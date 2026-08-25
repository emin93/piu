use std::{path::Path, sync::Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::database::{Database, DatabaseError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRoute {
    provider_id: String,
    model_id: String,
}

impl ModelRoute {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, RuntimePreferencesError> {
        let route = Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
        };
        route.validate()?;
        Ok(route)
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn selection(&self, effort: Option<&str>) -> ModelSelection {
        ModelSelection {
            route: self.clone(),
            effort: effort.map(ToOwned::to_owned),
        }
    }

    fn validate(&self) -> Result<(), RuntimePreferencesError> {
        if self.provider_id.is_empty() || self.model_id.is_empty() {
            Err(RuntimePreferencesError::InvalidModelRoute)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSelection {
    pub route: ModelRoute,
    pub effort: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceScope {
    Global,
    Project(i64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeResource {
    ModelRoute(ModelRoute),
    Skill(String),
    Extension(String),
    Package(String),
}

impl RuntimeResource {
    pub fn model_route(route: ModelRoute) -> Self {
        Self::ModelRoute(route)
    }

    pub fn skill(source_id: impl Into<String>) -> Self {
        Self::Skill(source_id.into())
    }

    pub fn extension(source_id: impl Into<String>) -> Self {
        Self::Extension(source_id.into())
    }

    pub fn package(source_id: impl Into<String>) -> Self {
        Self::Package(source_id.into())
    }

    fn storage_identity(&self) -> Result<(&'static str, &str, &str), RuntimePreferencesError> {
        let identity = match self {
            Self::ModelRoute(route) => {
                route.validate()?;
                ("model_route", route.provider_id(), route.model_id())
            }
            Self::Skill(source_id) => ("skill", "", source_id.as_str()),
            Self::Extension(source_id) => ("extension", "", source_id.as_str()),
            Self::Package(source_id) => ("package", "", source_id.as_str()),
        };
        if identity.2.is_empty() {
            Err(RuntimePreferencesError::InvalidResourceIdentity)
        } else {
            Ok(identity)
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimePreferencesError {
    #[error("model routes require non-empty provider and model identifiers")]
    InvalidModelRoute,
    #[error("reasoning effort requires a non-empty Pi level identifier")]
    InvalidEffort,
    #[error("runtime resources require a non-empty stable source identifier")]
    InvalidResourceIdentity,
    #[error("project resource scope requires a positive project identifier")]
    InvalidProjectId,
    #[error("runtime preferences lock is poisoned")]
    LockPoisoned,
    #[error(transparent)]
    Database(#[from] DatabaseError),
}

pub struct RuntimePreferences {
    database: Mutex<Database>,
}

impl RuntimePreferences {
    pub fn open(database_path: &Path) -> Result<Self, RuntimePreferencesError> {
        Ok(Self {
            database: Mutex::new(Database::open(database_path)?),
        })
    }

    pub fn select_route(
        &self,
        route: &ModelRoute,
    ) -> Result<ModelSelection, RuntimePreferencesError> {
        route.validate()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| RuntimePreferencesError::LockPoisoned)?;
        let transaction = database
            .connection_mut()
            .transaction()
            .map_err(DatabaseError::Query)?;
        transaction
            .execute(
                "INSERT INTO runtime_model_selection (singleton, provider_id, model_id)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(singleton) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    model_id = excluded.model_id",
                params![route.provider_id(), route.model_id()],
            )
            .map_err(DatabaseError::Query)?;
        let effort = remembered_effort(&transaction, route)?;
        transaction.commit().map_err(DatabaseError::Query)?;
        Ok(ModelSelection {
            route: route.clone(),
            effort,
        })
    }

    pub fn remember_effort(
        &self,
        route: &ModelRoute,
        effort: &str,
    ) -> Result<(), RuntimePreferencesError> {
        route.validate()?;
        if effort.is_empty() {
            return Err(RuntimePreferencesError::InvalidEffort);
        }
        self.with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO model_route_efforts (provider_id, model_id, effort)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(provider_id, model_id) DO UPDATE SET effort = excluded.effort",
                    params![route.provider_id(), route.model_id(), effort],
                )
                .map_err(DatabaseError::Query)?;
            Ok(())
        })
    }

    pub fn select_route_with_effort(
        &self,
        route: &ModelRoute,
        effort: &str,
    ) -> Result<ModelSelection, RuntimePreferencesError> {
        route.validate()?;
        if effort.is_empty() {
            return Err(RuntimePreferencesError::InvalidEffort);
        }
        let mut database = self
            .database
            .lock()
            .map_err(|_| RuntimePreferencesError::LockPoisoned)?;
        let transaction = database
            .connection_mut()
            .transaction()
            .map_err(DatabaseError::Query)?;
        transaction
            .execute(
                "INSERT INTO model_route_efforts (provider_id, model_id, effort)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(provider_id, model_id) DO UPDATE SET effort = excluded.effort",
                params![route.provider_id(), route.model_id(), effort],
            )
            .map_err(DatabaseError::Query)?;
        transaction
            .execute(
                "INSERT INTO runtime_model_selection (singleton, provider_id, model_id)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(singleton) DO UPDATE SET
                    provider_id = excluded.provider_id,
                    model_id = excluded.model_id",
                params![route.provider_id(), route.model_id()],
            )
            .map_err(DatabaseError::Query)?;
        transaction.commit().map_err(DatabaseError::Query)?;
        Ok(route.selection(Some(effort)))
    }

    pub fn remembered_effort(
        &self,
        route: &ModelRoute,
    ) -> Result<Option<String>, RuntimePreferencesError> {
        route.validate()?;
        self.with_connection(|connection| remembered_effort(connection, route))
    }

    pub fn current_selection(&self) -> Result<Option<ModelSelection>, RuntimePreferencesError> {
        self.with_connection(|connection| Ok(current_selection(connection)?))
    }

    pub fn initial_chat_selection(
        &self,
        chat_id: &str,
    ) -> Result<Option<ModelSelection>, RuntimePreferencesError> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT initial_model_provider, initial_model_id, initial_reasoning_effort
                     FROM chats WHERE id = ?1",
                    [chat_id],
                    |row| {
                        let provider_id: Option<String> = row.get(0)?;
                        let model_id: Option<String> = row.get(1)?;
                        let effort: Option<String> = row.get(2)?;
                        Ok((provider_id, model_id, effort))
                    },
                )
                .optional()
                .map_err(DatabaseError::Query)?
                .map(decode_selection)
                .transpose()
                .map(Option::flatten)
                .map_err(RuntimePreferencesError::Database)
        })
    }

    pub fn set_resource_enabled(
        &self,
        scope: ResourceScope,
        resource: &RuntimeResource,
        enabled: bool,
    ) -> Result<(), RuntimePreferencesError> {
        let (kind, provider_id, resource_id) = resource.storage_identity()?;
        self.with_connection(|connection| {
            match scope {
                ResourceScope::Global => connection.execute(
                    "INSERT INTO global_resource_enable_overrides (
                       resource_kind, provider_id, resource_id, enabled
                     ) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(resource_kind, provider_id, resource_id)
                     DO UPDATE SET enabled = excluded.enabled",
                    params![kind, provider_id, resource_id, enabled],
                ),
                ResourceScope::Project(project_id) => {
                    validate_project_id(project_id)?;
                    connection.execute(
                        "INSERT INTO project_resource_enable_overrides (
                           project_id, resource_kind, provider_id, resource_id, enabled
                         ) VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(project_id, resource_kind, provider_id, resource_id)
                         DO UPDATE SET enabled = excluded.enabled",
                        params![project_id, kind, provider_id, resource_id, enabled],
                    )
                }
            }
            .map_err(DatabaseError::Query)?;
            Ok(())
        })
    }

    pub fn set_model_route_enabled(
        &self,
        scope: ResourceScope,
        route: &ModelRoute,
        enabled: bool,
        fallback: Option<&ModelSelection>,
    ) -> Result<(), RuntimePreferencesError> {
        route.validate()?;
        let fallback = fallback
            .map(|selection| {
                selection.route.validate()?;
                let effort = selection
                    .effort
                    .as_deref()
                    .filter(|effort| !effort.is_empty())
                    .ok_or(RuntimePreferencesError::InvalidEffort)?;
                Ok::<(&ModelRoute, &str), RuntimePreferencesError>((&selection.route, effort))
            })
            .transpose()?;
        let resource = RuntimeResource::model_route(route.clone());
        let (kind, provider_id, resource_id) = resource.storage_identity()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| RuntimePreferencesError::LockPoisoned)?;
        let transaction = database
            .connection_mut()
            .transaction()
            .map_err(DatabaseError::Query)?;
        match scope {
            ResourceScope::Global => transaction.execute(
                "INSERT INTO global_resource_enable_overrides (
                   resource_kind, provider_id, resource_id, enabled
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(resource_kind, provider_id, resource_id)
                 DO UPDATE SET enabled = excluded.enabled",
                params![kind, provider_id, resource_id, enabled],
            ),
            ResourceScope::Project(project_id) => {
                validate_project_id(project_id)?;
                transaction.execute(
                    "INSERT INTO project_resource_enable_overrides (
                       project_id, resource_kind, provider_id, resource_id, enabled
                     ) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(project_id, resource_kind, provider_id, resource_id)
                     DO UPDATE SET enabled = excluded.enabled",
                    params![project_id, kind, provider_id, resource_id, enabled],
                )
            }
        }
        .map_err(DatabaseError::Query)?;
        if let Some((fallback_route, effort)) = fallback {
            transaction
                .execute(
                    "INSERT INTO model_route_efforts (provider_id, model_id, effort)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(provider_id, model_id) DO UPDATE SET effort = excluded.effort",
                    params![
                        fallback_route.provider_id(),
                        fallback_route.model_id(),
                        effort
                    ],
                )
                .map_err(DatabaseError::Query)?;
            transaction
                .execute(
                    "INSERT INTO runtime_model_selection (singleton, provider_id, model_id)
                     VALUES (1, ?1, ?2)
                     ON CONFLICT(singleton) DO UPDATE SET
                        provider_id = excluded.provider_id,
                        model_id = excluded.model_id",
                    params![fallback_route.provider_id(), fallback_route.model_id()],
                )
                .map_err(DatabaseError::Query)?;
        }
        transaction.commit().map_err(DatabaseError::Query)?;
        Ok(())
    }

    pub fn resource_enabled(
        &self,
        scope: ResourceScope,
        resource: &RuntimeResource,
    ) -> Result<Option<bool>, RuntimePreferencesError> {
        let (kind, provider_id, resource_id) = resource.storage_identity()?;
        self.with_connection(|connection| {
            let enabled = match scope {
                ResourceScope::Global => connection
                    .query_row(
                        "SELECT enabled FROM global_resource_enable_overrides
                         WHERE resource_kind = ?1 AND provider_id = ?2 AND resource_id = ?3",
                        params![kind, provider_id, resource_id],
                        |row| row.get(0),
                    )
                    .optional(),
                ResourceScope::Project(project_id) => {
                    validate_project_id(project_id)?;
                    connection
                        .query_row(
                            "SELECT enabled FROM project_resource_enable_overrides
                             WHERE project_id = ?1 AND resource_kind = ?2
                               AND provider_id = ?3 AND resource_id = ?4",
                            params![project_id, kind, provider_id, resource_id],
                            |row| row.get(0),
                        )
                        .optional()
                }
            }
            .map_err(DatabaseError::Query)?;
            Ok(enabled)
        })
    }

    pub fn clear_resource_override(
        &self,
        scope: ResourceScope,
        resource: &RuntimeResource,
    ) -> Result<(), RuntimePreferencesError> {
        let (kind, provider_id, resource_id) = resource.storage_identity()?;
        self.with_connection(|connection| {
            match scope {
                ResourceScope::Global => connection.execute(
                    "DELETE FROM global_resource_enable_overrides
                     WHERE resource_kind = ?1 AND provider_id = ?2 AND resource_id = ?3",
                    params![kind, provider_id, resource_id],
                ),
                ResourceScope::Project(project_id) => {
                    validate_project_id(project_id)?;
                    connection.execute(
                        "DELETE FROM project_resource_enable_overrides
                         WHERE project_id = ?1 AND resource_kind = ?2
                           AND provider_id = ?3 AND resource_id = ?4",
                        params![project_id, kind, provider_id, resource_id],
                    )
                }
            }
            .map_err(DatabaseError::Query)?;
            Ok(())
        })
    }

    fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, RuntimePreferencesError>,
    ) -> Result<T, RuntimePreferencesError> {
        let database = self
            .database
            .lock()
            .map_err(|_| RuntimePreferencesError::LockPoisoned)?;
        operation(database.connection())
    }
}

pub(crate) fn current_selection(
    connection: &Connection,
) -> Result<Option<ModelSelection>, DatabaseError> {
    connection
        .query_row(
            "SELECT selection.provider_id, selection.model_id, efforts.effort
             FROM runtime_model_selection AS selection
             LEFT JOIN model_route_efforts AS efforts
               ON efforts.provider_id = selection.provider_id
              AND efforts.model_id = selection.model_id
             WHERE selection.singleton = 1",
            [],
            |row| {
                Ok((
                    Some(row.get::<_, String>(0)?),
                    Some(row.get::<_, String>(1)?),
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(DatabaseError::Query)?
        .map(decode_selection)
        .transpose()
        .map(Option::flatten)
}

fn remembered_effort(
    connection: &Connection,
    route: &ModelRoute,
) -> Result<Option<String>, RuntimePreferencesError> {
    connection
        .query_row(
            "SELECT effort FROM model_route_efforts WHERE provider_id = ?1 AND model_id = ?2",
            params![route.provider_id(), route.model_id()],
            |row| row.get(0),
        )
        .optional()
        .map_err(DatabaseError::Query)
        .map_err(RuntimePreferencesError::Database)
}

fn decode_selection(
    stored: (Option<String>, Option<String>, Option<String>),
) -> Result<Option<ModelSelection>, DatabaseError> {
    match stored {
        (None, None, None) => Ok(None),
        (Some(provider_id), Some(model_id), effort) => Ok(Some(ModelSelection {
            route: ModelRoute::new(provider_id, model_id)
                .map_err(|_| DatabaseError::Query(rusqlite::Error::InvalidQuery))?,
            effort,
        })),
        _ => Err(DatabaseError::Query(rusqlite::Error::InvalidQuery)),
    }
}

fn validate_project_id(project_id: i64) -> Result<(), RuntimePreferencesError> {
    if project_id > 0 {
        Ok(())
    } else {
        Err(RuntimePreferencesError::InvalidProjectId)
    }
}
