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

#[derive(Clone, Debug)]
pub(crate) struct ResourcePreferenceCheckpoint {
    scope: ResourceScope,
    resource: RuntimeResource,
    enabled: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeResource {
    ModelRoute(ModelRoute),
    Skill(String),
    Extension(String),
    Package(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceEnableOverride {
    pub scope: ResourceScope,
    pub resource: RuntimeResource,
    pub enabled: bool,
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
    ) -> Result<(), RuntimePreferencesError> {
        route.validate()?;
        let resource = RuntimeResource::model_route(route.clone());
        self.set_resource_enabled(scope, &resource, enabled)
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

    pub(crate) fn inspector_resource_overrides(
        &self,
        project_id: i64,
    ) -> Result<Vec<ResourceEnableOverride>, RuntimePreferencesError> {
        validate_project_id(project_id)?;
        self.with_connection(|connection| {
            let mut overrides = Vec::new();
            let mut global = connection
                .prepare(
                    "SELECT resource_kind, resource_id, enabled
                     FROM global_resource_enable_overrides
                     WHERE resource_kind IN ('extension', 'package')
                     ORDER BY resource_kind, resource_id",
                )
                .map_err(DatabaseError::Query)?;
            let rows = global
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })
                .map_err(DatabaseError::Query)?;
            for row in rows {
                let (kind, id, enabled) = row.map_err(DatabaseError::Query)?;
                overrides.push(ResourceEnableOverride {
                    scope: ResourceScope::Global,
                    resource: inspector_resource(kind, id)?,
                    enabled,
                });
            }

            let mut project = connection
                .prepare(
                    "SELECT resource_kind, resource_id, enabled
                     FROM project_resource_enable_overrides
                     WHERE project_id = ?1 AND resource_kind IN ('extension', 'package')
                     ORDER BY resource_kind, resource_id",
                )
                .map_err(DatabaseError::Query)?;
            let rows = project
                .query_map([project_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                })
                .map_err(DatabaseError::Query)?;
            for row in rows {
                let (kind, id, enabled) = row.map_err(DatabaseError::Query)?;
                overrides.push(ResourceEnableOverride {
                    scope: ResourceScope::Project(project_id),
                    resource: inspector_resource(kind, id)?,
                    enabled,
                });
            }
            Ok(overrides)
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

    pub(crate) fn checkpoint_resource_change(
        &self,
        scope: ResourceScope,
        resource: &RuntimeResource,
    ) -> Result<ResourcePreferenceCheckpoint, RuntimePreferencesError> {
        let enabled = self.resource_enabled(scope, resource)?;
        Ok(ResourcePreferenceCheckpoint {
            scope,
            resource: resource.clone(),
            enabled,
        })
    }

    pub(crate) fn restore_resource_change(
        &self,
        checkpoint: ResourcePreferenceCheckpoint,
    ) -> Result<(), RuntimePreferencesError> {
        let (kind, provider_id, resource_id) = checkpoint.resource.storage_identity()?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| RuntimePreferencesError::LockPoisoned)?;
        let transaction = database
            .connection_mut()
            .transaction()
            .map_err(DatabaseError::Query)?;
        match (checkpoint.scope, checkpoint.enabled) {
            (ResourceScope::Global, Some(enabled)) => transaction.execute(
                "INSERT INTO global_resource_enable_overrides (
                   resource_kind, provider_id, resource_id, enabled
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(resource_kind, provider_id, resource_id)
                 DO UPDATE SET enabled = excluded.enabled",
                params![kind, provider_id, resource_id, enabled],
            ),
            (ResourceScope::Global, None) => transaction.execute(
                "DELETE FROM global_resource_enable_overrides
                 WHERE resource_kind = ?1 AND provider_id = ?2 AND resource_id = ?3",
                params![kind, provider_id, resource_id],
            ),
            (ResourceScope::Project(project_id), Some(enabled)) => {
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
            (ResourceScope::Project(project_id), None) => {
                validate_project_id(project_id)?;
                transaction.execute(
                    "DELETE FROM project_resource_enable_overrides
                     WHERE project_id = ?1 AND resource_kind = ?2
                       AND provider_id = ?3 AND resource_id = ?4",
                    params![project_id, kind, provider_id, resource_id],
                )
            }
        }
        .map_err(DatabaseError::Query)?;

        transaction.commit().map_err(DatabaseError::Query)?;
        Ok(())
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

fn inspector_resource(
    kind: String,
    id: String,
) -> Result<RuntimeResource, RuntimePreferencesError> {
    match kind.as_str() {
        "extension" => Ok(RuntimeResource::extension(id)),
        "package" => Ok(RuntimeResource::package(id)),
        _ => Err(RuntimePreferencesError::InvalidResourceIdentity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_change_rollback_preserves_a_concurrent_model_selection() {
        let root = tempfile::TempDir::new().unwrap();
        let database_path = root.path().join("piu.sqlite3");
        let preferences = RuntimePreferences::open(&database_path).unwrap();
        let chat_preferences = RuntimePreferences::open(&database_path).unwrap();
        let codex = ModelRoute::new("openai-codex", "gpt-5.6-sol").unwrap();
        let qwen = ModelRoute::new("local-mlx", "qwen3.8-27b").unwrap();
        preferences
            .select_route_with_effort(&codex, "high")
            .unwrap();
        let resource = RuntimeResource::model_route(codex.clone());
        let checkpoint = preferences
            .checkpoint_resource_change(ResourceScope::Global, &resource)
            .unwrap();

        preferences
            .set_model_route_enabled(ResourceScope::Global, &codex, false)
            .unwrap();
        chat_preferences
            .select_route_with_effort(&qwen, "xhigh")
            .unwrap();
        preferences.restore_resource_change(checkpoint).unwrap();

        assert_eq!(
            preferences
                .resource_enabled(ResourceScope::Global, &resource)
                .unwrap(),
            None
        );
        assert_eq!(
            preferences.current_selection().unwrap(),
            Some(qwen.selection(Some("xhigh")))
        );
        assert_eq!(
            preferences.remembered_effort(&qwen).unwrap().as_deref(),
            Some("xhigh")
        );
    }
}
