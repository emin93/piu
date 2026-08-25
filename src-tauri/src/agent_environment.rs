use std::{
    collections::{BTreeMap, HashSet},
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Mutex as AsyncMutex,
    time::timeout,
};
use ts_rs::TS;

use crate::{
    chat_runtime_host::{ModelControlsSnapshot, ModelRouteId, ModelRouteSummary, ReasoningEffort},
    owned_process::{OwnedProcessGroup, spawn_owned_piped_process},
    project_inbox::{ProjectInbox, ProjectInboxError},
    runtime_preferences::{
        ModelRoute, ResourceScope, RuntimePreferences, RuntimePreferencesError, RuntimeResource,
    },
};

#[derive(Clone, Debug)]
pub struct AgentEnvironmentProcessSpec {
    pub executable: PathBuf,
    pub launcher: PathBuf,
    pub agent_directory: PathBuf,
    pub credential_lock_directory: PathBuf,
    pub environment: BTreeMap<OsString, OsString>,
}

impl AgentEnvironmentProcessSpec {
    pub fn from_bundled_runtime(
        resource_directory: &Path,
        application_data_directory: &Path,
        real_home_directory: &Path,
    ) -> Self {
        let runtime = resource_directory.join("agent-runtime");
        let pi = runtime.join("pi");
        let git = resource_directory.join("git");
        let git_bin = git.join("bin");
        let path = env::join_paths([git_bin.as_path(), Path::new("/usr/bin"), Path::new("/bin")])
            .expect("fixed bundled Git paths must form a valid PATH");
        let mut environment = BTreeMap::new();
        environment.insert(
            OsString::from("HOME"),
            real_home_directory.as_os_str().to_owned(),
        );
        environment.insert(OsString::from("PATH"), path);
        environment.insert(
            OsString::from("GIT_EXEC_PATH"),
            git.join("libexec/git-core").into_os_string(),
        );
        environment.insert(
            OsString::from("GIT_TEMPLATE_DIR"),
            git.join("share/git-core/templates").into_os_string(),
        );
        environment.insert(OsString::from("LC_ALL"), OsString::from("C"));
        environment.insert(OsString::from("PI_SKIP_VERSION_CHECK"), OsString::from("1"));
        environment.insert(OsString::from("PI_TELEMETRY"), OsString::from("0"));
        Self {
            executable: runtime.join("node/bin/node"),
            launcher: pi.join("launcher/environment-launcher.mjs"),
            agent_directory: application_data_directory.join("agent"),
            credential_lock_directory: application_data_directory.join("credential-locks"),
            environment,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentEnvironmentPolicy {
    pub inspection_timeout: Duration,
    pub maximum_stdout_bytes: usize,
    pub maximum_stderr_bytes: usize,
}

impl Default for AgentEnvironmentPolicy {
    fn default() -> Self {
        Self {
            inspection_timeout: Duration::from_secs(30),
            maximum_stdout_bytes: 4 * 1024 * 1024,
            maximum_stderr_bytes: 64 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentResourceSource {
    Piu,
    Project,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentResourceOrigin {
    Package,
    TopLevel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentResourceSummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source: AgentResourceSource,
    pub origin: AgentResourceOrigin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentPackageSummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source: AgentResourceSource,
    pub filtered: bool,
    pub installed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentEnvironmentResources {
    pub extensions: Vec<AgentResourceSummary>,
    pub skills: Vec<AgentResourceSummary>,
    pub packages: Vec<AgentPackageSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentModelRoute {
    pub id: ModelRouteId,
    pub name: String,
    pub accepts_images: bool,
    pub thinking_levels: Vec<ReasoningEffort>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentEnvironmentDiagnosticResource {
    Package,
    Settings,
    Runtime,
    Extension,
    Skill,
    Model,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentEnvironmentDiagnosticKind {
    Info,
    Warning,
    Error,
    Collision,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentEnvironmentDiagnostic {
    pub resource: AgentEnvironmentDiagnosticResource,
    pub kind: AgentEnvironmentDiagnosticKind,
    pub message: String,
    pub path: Option<String>,
    pub source: Option<String>,
    pub source_scope: Option<AgentResourceSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentEnvironmentSnapshot {
    pub model_controls: ModelControlsSnapshot,
    pub model_routes: Vec<AgentModelRoute>,
    pub resources: AgentEnvironmentResources,
    pub diagnostics: Vec<AgentEnvironmentDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLaunchResources {
    pub extension_paths: Vec<PathBuf>,
    pub skill_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentResourceId {
    ModelRoute { route: ModelRouteId },
    Skill { id: String },
    Extension { id: String },
    Package { id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub enum AgentResourcePreferenceScope {
    Global,
    Project,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct AgentResourcePreferenceChange {
    pub scope: AgentResourcePreferenceScope,
    pub resource: AgentResourceId,
    pub enabled: bool,
}

#[derive(Debug, Error)]
pub enum AgentEnvironmentError {
    #[error("agent environment paths must be absolute")]
    NonAbsoluteProcessPath,
    #[error("agent environment inspection requires the user's HOME directory")]
    MissingHome,
    #[error("agent environment limits must be greater than zero")]
    InvalidPolicy,
    #[error("could not prepare agent environment runtime state")]
    RuntimeStorage,
    #[error("could not start the agent environment inspector")]
    Spawn,
    #[error("agent environment inspection timed out")]
    TimedOut,
    #[error("agent environment inspection exceeded its output limit")]
    OutputLimitExceeded,
    #[error("agent environment inspection failed")]
    ChildFailed,
    #[error("agent environment inspection returned invalid data")]
    InvalidSnapshot,
    #[error("Pi reported no enabled model routes")]
    NoAvailableModelRoutes,
    #[error("at least one model route must remain enabled")]
    CannotDisableLastModelRoute,
    #[error("model route {provider}/{model_id} is unavailable")]
    ModelUnavailable { provider: String, model_id: String },
    #[error("reasoning effort is unavailable for the selected route")]
    EffortUnavailable { effort: ReasoningEffort },
    #[error("resource is unavailable in this project environment")]
    ResourceUnavailable,
    #[error(transparent)]
    Project(#[from] ProjectInboxError),
    #[error(transparent)]
    Preferences(#[from] RuntimePreferencesError),
}

pub struct AgentEnvironment {
    projects: Arc<ProjectInbox>,
    preferences: Arc<RuntimePreferences>,
    process: AgentEnvironmentProcessSpec,
    policy: AgentEnvironmentPolicy,
    preference_change: AsyncMutex<()>,
}

impl AgentEnvironment {
    pub fn production(
        projects: Arc<ProjectInbox>,
        database_path: &Path,
        application_data_directory: &Path,
        resource_directory: &Path,
        real_home_directory: &Path,
    ) -> Result<Self, AgentEnvironmentError> {
        Self::new(
            projects,
            Arc::new(RuntimePreferences::open(database_path)?),
            AgentEnvironmentProcessSpec::from_bundled_runtime(
                resource_directory,
                application_data_directory,
                real_home_directory,
            ),
            AgentEnvironmentPolicy::default(),
        )
    }

    pub fn new(
        projects: Arc<ProjectInbox>,
        preferences: Arc<RuntimePreferences>,
        process: AgentEnvironmentProcessSpec,
        policy: AgentEnvironmentPolicy,
    ) -> Result<Self, AgentEnvironmentError> {
        validate(&process, &policy)?;
        Ok(Self {
            projects,
            preferences,
            process,
            policy,
            preference_change: AsyncMutex::new(()),
        })
    }

    pub async fn snapshot(
        &self,
        project_id: i64,
    ) -> Result<AgentEnvironmentSnapshot, AgentEnvironmentError> {
        let discovered = self.discover(project_id).await?;
        self.materialize(project_id, discovered)
    }

    pub async fn model_controls(
        &self,
        project_id: i64,
    ) -> Result<ModelControlsSnapshot, AgentEnvironmentError> {
        Ok(self.snapshot(project_id).await?.model_controls)
    }

    pub async fn launch_resources(
        &self,
        project_id: i64,
    ) -> Result<AgentLaunchResources, AgentEnvironmentError> {
        let discovered = self.discover(project_id).await?;
        let extension_paths = discovered
            .resources
            .extensions
            .iter()
            .map(|resource| {
                self.discovered_resource_enabled(project_id, resource, ResourceKind::Extension)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        let skill_paths = discovered
            .resources
            .skills
            .iter()
            .map(|resource| {
                self.discovered_resource_enabled(project_id, resource, ResourceKind::Skill)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(AgentLaunchResources {
            extension_paths,
            skill_paths,
        })
    }

    pub async fn select_model_route(
        &self,
        project_id: i64,
        route: ModelRouteId,
    ) -> Result<ModelControlsSnapshot, AgentEnvironmentError> {
        let _change = self.preference_change.lock().await;
        let discovered = self.discover(project_id).await?;
        let available = self.materialize(project_id, discovered)?;
        let selected = available
            .model_routes
            .iter()
            .find(|candidate| candidate.enabled && candidate.id == route)
            .ok_or_else(|| AgentEnvironmentError::ModelUnavailable {
                provider: route.provider.clone(),
                model_id: route.model_id.clone(),
            })?;
        let persisted = persisted_route(&selected.id)?;
        let effort = self
            .preferences
            .remembered_effort(&persisted)?
            .as_deref()
            .and_then(reasoning_effort)
            .filter(|effort| selected.thinking_levels.contains(effort))
            .unwrap_or(selected.thinking_levels[0]);
        self.preferences
            .select_route_with_effort(&persisted, effort_as_pi(effort))?;
        let mut controls = controls_for(&available.model_routes, &selected.id, effort)?;
        controls.applies_after_current_step = false;
        Ok(controls)
    }

    pub async fn select_reasoning_effort(
        &self,
        project_id: i64,
        effort: ReasoningEffort,
    ) -> Result<ModelControlsSnapshot, AgentEnvironmentError> {
        let _change = self.preference_change.lock().await;
        let discovered = self.discover(project_id).await?;
        let available = self.materialize(project_id, discovered)?;
        let selected = available
            .model_routes
            .iter()
            .find(|route| route.id == available.model_controls.selected_route)
            .ok_or(AgentEnvironmentError::NoAvailableModelRoutes)?;
        if !selected.thinking_levels.contains(&effort) {
            return Err(AgentEnvironmentError::EffortUnavailable { effort });
        }
        self.preferences
            .select_route_with_effort(&persisted_route(&selected.id)?, effort_as_pi(effort))?;
        controls_for(&available.model_routes, &selected.id, effort)
    }

    pub async fn set_resource_enabled(
        &self,
        project_id: i64,
        scope: AgentResourcePreferenceScope,
        resource: AgentResourceId,
        enabled: bool,
    ) -> Result<AgentResourcePreferenceChange, AgentEnvironmentError> {
        let _change = self.preference_change.lock().await;
        let discovered = self.discover(project_id).await?;
        if !discovered.contains(&resource) {
            return Err(AgentEnvironmentError::ResourceUnavailable);
        }
        let persisted_scope = match scope {
            AgentResourcePreferenceScope::Global => ResourceScope::Global,
            AgentResourcePreferenceScope::Project => ResourceScope::Project(project_id),
        };
        if let AgentResourceId::ModelRoute { route } = &resource {
            let available = self.materialize(project_id, discovered)?;
            let target = available
                .model_routes
                .iter()
                .find(|candidate| candidate.id == *route)
                .ok_or(AgentEnvironmentError::ResourceUnavailable)?;
            let target_persisted = persisted_route(route)?;
            let persisted_selection_is_target = self
                .preferences
                .current_selection()?
                .is_some_and(|selection| selection.route == target_persisted);
            let fallback = if !enabled
                && (persisted_selection_is_target
                    || available.model_controls.selected_route == target.id)
            {
                let mut fallback = None;
                for candidate in &available.model_routes {
                    if candidate.id != target.id
                        && self.route_enabled_for_scope(project_id, candidate, scope)?
                    {
                        fallback = Some(candidate);
                        break;
                    }
                }
                let fallback =
                    fallback.ok_or(AgentEnvironmentError::CannotDisableLastModelRoute)?;
                let fallback_route = persisted_route(&fallback.id)?;
                let effort = self
                    .preferences
                    .remembered_effort(&fallback_route)?
                    .as_deref()
                    .and_then(reasoning_effort)
                    .filter(|effort| fallback.thinking_levels.contains(effort))
                    .unwrap_or(fallback.thinking_levels[0]);
                Some(fallback_route.selection(Some(effort_as_pi(effort))))
            } else {
                None
            };
            self.preferences.set_model_route_enabled(
                persisted_scope,
                &target_persisted,
                enabled,
                fallback.as_ref(),
            )?;
            return Ok(AgentResourcePreferenceChange {
                scope,
                resource,
                enabled,
            });
        }
        let persisted_resource = persisted_resource(&resource)?;
        self.preferences
            .set_resource_enabled(persisted_scope, &persisted_resource, enabled)?;
        Ok(AgentResourcePreferenceChange {
            scope,
            resource,
            enabled,
        })
    }

    async fn discover(
        &self,
        project_id: i64,
    ) -> Result<DiscoveredEnvironment, AgentEnvironmentError> {
        let projects = Arc::clone(&self.projects);
        let location = tokio::task::spawn_blocking(move || projects.project_location(project_id))
            .await
            .map_err(|_| AgentEnvironmentError::RuntimeStorage)??;
        run_inspector(&self.process, &self.policy, &location.canonical_path).await
    }

    fn materialize(
        &self,
        project_id: i64,
        discovered: DiscoveredEnvironment,
    ) -> Result<AgentEnvironmentSnapshot, AgentEnvironmentError> {
        let mut model_routes = discovered
            .model_routes
            .into_iter()
            .map(|route| self.model_route(project_id, route))
            .collect::<Result<Vec<_>, _>>()?;
        validate_unique_model_routes(&model_routes)?;
        let enabled_routes = model_routes
            .iter()
            .filter(|route| route.enabled)
            .collect::<Vec<_>>();
        let selected_route = self
            .preferences
            .current_selection()?
            .and_then(|selection| {
                enabled_routes.iter().find(|route| {
                    route.id.provider == selection.route.provider_id()
                        && route.id.model_id == selection.route.model_id()
                })
            })
            .copied()
            .or_else(|| enabled_routes.first().copied())
            .ok_or(AgentEnvironmentError::NoAvailableModelRoutes)?;
        let selected_persisted = persisted_route(&selected_route.id)?;
        let selected_effort = self
            .preferences
            .remembered_effort(&selected_persisted)?
            .as_deref()
            .and_then(reasoning_effort)
            .filter(|effort| selected_route.thinking_levels.contains(effort))
            .unwrap_or(selected_route.thinking_levels[0]);
        let model_controls = controls_for(&model_routes, &selected_route.id, selected_effort)?;
        let extensions = discovered
            .resources
            .extensions
            .into_iter()
            .map(|resource| self.resource(project_id, resource, ResourceKind::Extension))
            .collect::<Result<Vec<_>, _>>()?;
        let skills = discovered
            .resources
            .skills
            .into_iter()
            .map(|resource| self.resource(project_id, resource, ResourceKind::Skill))
            .collect::<Result<Vec<_>, _>>()?;
        let packages = discovered
            .resources
            .packages
            .into_iter()
            .map(|resource| self.package(project_id, resource))
            .collect::<Result<Vec<_>, _>>()?;
        let diagnostics = discovered
            .diagnostics
            .into_iter()
            .map(AgentEnvironmentDiagnostic::from)
            .collect();
        model_routes.shrink_to_fit();
        Ok(AgentEnvironmentSnapshot {
            model_controls,
            model_routes,
            resources: AgentEnvironmentResources {
                extensions,
                skills,
                packages,
            },
            diagnostics,
        })
    }

    fn model_route(
        &self,
        project_id: i64,
        route: DiscoveredModelRoute,
    ) -> Result<AgentModelRoute, AgentEnvironmentError> {
        if route.provider.is_empty() || route.id.is_empty() || route.name.is_empty() {
            return Err(AgentEnvironmentError::InvalidSnapshot);
        }
        let thinking_levels = route
            .thinking_levels
            .iter()
            .map(|level| reasoning_effort(level).ok_or(AgentEnvironmentError::InvalidSnapshot))
            .collect::<Result<Vec<_>, _>>()?;
        if thinking_levels.is_empty()
            || thinking_levels.iter().collect::<HashSet<_>>().len() != thinking_levels.len()
        {
            return Err(AgentEnvironmentError::InvalidSnapshot);
        }
        let id = ModelRouteId {
            provider: route.provider,
            model_id: route.id,
        };
        let enabled = self.effective_enabled(
            project_id,
            &RuntimeResource::model_route(persisted_route(&id)?),
            None,
            true,
        )?;
        Ok(AgentModelRoute {
            id,
            name: route.name,
            accepts_images: route.accepts_images,
            thinking_levels,
            enabled,
        })
    }

    fn resource(
        &self,
        project_id: i64,
        resource: DiscoveredResource,
        kind: ResourceKind,
    ) -> Result<AgentResourceSummary, AgentEnvironmentError> {
        if resource.id.is_empty() || resource.name.is_empty() {
            return Err(AgentEnvironmentError::InvalidSnapshot);
        }
        validate_resource_path(&resource.path)?;
        let persisted = match kind {
            ResourceKind::Skill => RuntimeResource::skill(&resource.id),
            ResourceKind::Extension => RuntimeResource::extension(&resource.id),
        };
        let package = (resource.origin == DiscoveredOrigin::Package)
            .then(|| RuntimeResource::package(&resource.source));
        Ok(AgentResourceSummary {
            id: resource.id,
            name: resource.name,
            enabled: self.effective_enabled(
                project_id,
                &persisted,
                package.as_ref(),
                resource.enabled,
            )?,
            source: resource.scope.into(),
            origin: resource.origin.into(),
        })
    }

    fn package(
        &self,
        project_id: i64,
        package: DiscoveredPackage,
    ) -> Result<AgentPackageSummary, AgentEnvironmentError> {
        if package.id.is_empty() || package.name.is_empty() || package.source.is_empty() {
            return Err(AgentEnvironmentError::InvalidSnapshot);
        }
        let persisted = RuntimeResource::package(&package.id);
        Ok(AgentPackageSummary {
            id: package.id,
            name: package.name,
            enabled: self.effective_enabled(project_id, &persisted, None, !package.filtered)?,
            source: package.scope.into(),
            filtered: package.filtered,
            installed: package.installed_path.is_some(),
        })
    }

    fn effective_enabled(
        &self,
        project_id: i64,
        resource: &RuntimeResource,
        package: Option<&RuntimeResource>,
        discovered: bool,
    ) -> Result<bool, AgentEnvironmentError> {
        let project = ResourceScope::Project(project_id);
        Ok(self
            .preferences
            .resource_enabled(project, resource)?
            .or(package
                .map(|package| self.preferences.resource_enabled(project, package))
                .transpose()?
                .flatten())
            .or(self
                .preferences
                .resource_enabled(ResourceScope::Global, resource)?)
            .or(package
                .map(|package| {
                    self.preferences
                        .resource_enabled(ResourceScope::Global, package)
                })
                .transpose()?
                .flatten())
            .unwrap_or(discovered))
    }

    fn discovered_resource_enabled(
        &self,
        project_id: i64,
        resource: &DiscoveredResource,
        kind: ResourceKind,
    ) -> Result<Option<PathBuf>, AgentEnvironmentError> {
        validate_resource_path(&resource.path)?;
        let persisted = match kind {
            ResourceKind::Skill => RuntimeResource::skill(&resource.id),
            ResourceKind::Extension => RuntimeResource::extension(&resource.id),
        };
        let package = (resource.origin == DiscoveredOrigin::Package)
            .then(|| RuntimeResource::package(&resource.source));
        self.effective_enabled(project_id, &persisted, package.as_ref(), resource.enabled)
            .map(|enabled| enabled.then(|| PathBuf::from(&resource.path)))
    }

    fn route_enabled_for_scope(
        &self,
        project_id: i64,
        route: &AgentModelRoute,
        scope: AgentResourcePreferenceScope,
    ) -> Result<bool, AgentEnvironmentError> {
        let resource = RuntimeResource::model_route(persisted_route(&route.id)?);
        match scope {
            AgentResourcePreferenceScope::Global => Ok(self
                .preferences
                .resource_enabled(ResourceScope::Global, &resource)?
                .unwrap_or(true)),
            AgentResourcePreferenceScope::Project => {
                self.effective_enabled(project_id, &resource, None, true)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum ResourceKind {
    Skill,
    Extension,
}

fn controls_for(
    routes: &[AgentModelRoute],
    selected_route: &ModelRouteId,
    selected_effort: ReasoningEffort,
) -> Result<ModelControlsSnapshot, AgentEnvironmentError> {
    let selected = routes
        .iter()
        .find(|route| route.enabled && route.id == *selected_route)
        .ok_or(AgentEnvironmentError::NoAvailableModelRoutes)?;
    if !selected.thinking_levels.contains(&selected_effort) {
        return Err(AgentEnvironmentError::EffortUnavailable {
            effort: selected_effort,
        });
    }
    Ok(ModelControlsSnapshot {
        routes: routes
            .iter()
            .filter(|route| route.enabled)
            .map(|route| ModelRouteSummary {
                id: route.id.clone(),
                name: route.name.clone(),
                accepts_images: route.accepts_images,
            })
            .collect(),
        selected_route: selected.id.clone(),
        efforts: selected.thinking_levels.clone(),
        selected_effort,
        applies_after_current_step: false,
    })
}

fn validate_unique_model_routes(routes: &[AgentModelRoute]) -> Result<(), AgentEnvironmentError> {
    let unique = routes.iter().map(|route| &route.id).collect::<HashSet<_>>();
    if unique.len() == routes.len() {
        Ok(())
    } else {
        Err(AgentEnvironmentError::InvalidSnapshot)
    }
}

fn validate_resource_path(path: &str) -> Result<(), AgentEnvironmentError> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(AgentEnvironmentError::InvalidSnapshot)
    }
}

fn persisted_route(route: &ModelRouteId) -> Result<ModelRoute, AgentEnvironmentError> {
    ModelRoute::new(&route.provider, &route.model_id).map_err(Into::into)
}

fn persisted_resource(
    resource: &AgentResourceId,
) -> Result<RuntimeResource, AgentEnvironmentError> {
    Ok(match resource {
        AgentResourceId::ModelRoute { route } => {
            RuntimeResource::model_route(persisted_route(route)?)
        }
        AgentResourceId::Skill { id } => RuntimeResource::skill(id),
        AgentResourceId::Extension { id } => RuntimeResource::extension(id),
        AgentResourceId::Package { id } => RuntimeResource::package(id),
    })
}

fn reasoning_effort(level: &str) -> Option<ReasoningEffort> {
    match level {
        "off" => Some(ReasoningEffort::Off),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::ExtraHigh),
        "max" => Some(ReasoningEffort::Maximum),
        _ => None,
    }
}

fn effort_as_pi(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Off => "off",
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::ExtraHigh => "xhigh",
        ReasoningEffort::Maximum => "max",
    }
}

fn validate(
    process: &AgentEnvironmentProcessSpec,
    policy: &AgentEnvironmentPolicy,
) -> Result<(), AgentEnvironmentError> {
    if !process.executable.is_absolute()
        || !process.launcher.is_absolute()
        || !process.agent_directory.is_absolute()
        || !process.credential_lock_directory.is_absolute()
    {
        return Err(AgentEnvironmentError::NonAbsoluteProcessPath);
    }
    if process
        .environment
        .get(OsStr::new("HOME"))
        .is_none_or(|home| home.is_empty())
    {
        return Err(AgentEnvironmentError::MissingHome);
    }
    if policy.inspection_timeout.is_zero()
        || policy.maximum_stdout_bytes == 0
        || policy.maximum_stderr_bytes == 0
    {
        return Err(AgentEnvironmentError::InvalidPolicy);
    }
    Ok(())
}

async fn run_inspector(
    process: &AgentEnvironmentProcessSpec,
    policy: &AgentEnvironmentPolicy,
    repository: &Path,
) -> Result<DiscoveredEnvironment, AgentEnvironmentError> {
    tokio::fs::create_dir_all(&process.credential_lock_directory)
        .await
        .map_err(|_| AgentEnvironmentError::RuntimeStorage)?;
    let arguments = vec![
        process.launcher.as_os_str().to_owned(),
        OsString::from("--cwd"),
        repository.as_os_str().to_owned(),
        OsString::from("--agent-dir"),
        process.agent_directory.as_os_str().to_owned(),
        OsString::from("--credential-lock-dir"),
        process.credential_lock_directory.as_os_str().to_owned(),
    ];
    let owned = spawn_owned_piped_process(
        &process.executable,
        &arguments,
        repository,
        &process.environment,
    )
    .map_err(|_| AgentEnvironmentError::Spawn)?;
    let process_group = owned.process_group;
    drop(owned.stdin);
    let stdout_reader = tokio::spawn(read_bounded(
        owned.stdout,
        policy.maximum_stdout_bytes,
        process_group,
    ));
    let stderr_reader = tokio::spawn(read_bounded(
        owned.stderr,
        policy.maximum_stderr_bytes,
        process_group,
    ));
    let mut child = owned.child;
    let (status, timed_out) = match timeout(policy.inspection_timeout, child.wait()).await {
        Ok(status) => (
            status.map_err(|_| AgentEnvironmentError::ChildFailed)?,
            false,
        ),
        Err(_) => {
            process_group.force_kill();
            let status = child
                .wait()
                .await
                .map_err(|_| AgentEnvironmentError::ChildFailed)?;
            (status, true)
        }
    };
    process_group.force_kill();
    let stdout = stdout_reader
        .await
        .map_err(|_| AgentEnvironmentError::ChildFailed)??;
    let _stderr = stderr_reader
        .await
        .map_err(|_| AgentEnvironmentError::ChildFailed)??;
    if timed_out {
        return Err(AgentEnvironmentError::TimedOut);
    }
    if !status.success() {
        return Err(AgentEnvironmentError::ChildFailed);
    }
    serde_json::from_slice(&stdout).map_err(|_| AgentEnvironmentError::InvalidSnapshot)
}

async fn read_bounded(
    mut reader: impl AsyncRead + Unpin,
    maximum_bytes: usize,
    process_group: OwnedProcessGroup,
) -> Result<Vec<u8>, AgentEnvironmentError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|_| AgentEnvironmentError::ChildFailed)?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > maximum_bytes {
            process_group.force_kill();
            return Err(AgentEnvironmentError::OutputLimitExceeded);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredEnvironment {
    model_routes: Vec<DiscoveredModelRoute>,
    resources: DiscoveredResources,
    diagnostics: Vec<DiscoveredDiagnostic>,
}

impl DiscoveredEnvironment {
    fn contains(&self, resource: &AgentResourceId) -> bool {
        match resource {
            AgentResourceId::ModelRoute { route } => self.model_routes.iter().any(|candidate| {
                candidate.provider == route.provider && candidate.id == route.model_id
            }),
            AgentResourceId::Skill { id } => self
                .resources
                .skills
                .iter()
                .any(|resource| resource.id == *id),
            AgentResourceId::Extension { id } => self
                .resources
                .extensions
                .iter()
                .any(|resource| resource.id == *id),
            AgentResourceId::Package { id } => self
                .resources
                .packages
                .iter()
                .any(|resource| resource.id == *id),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredModelRoute {
    provider: String,
    id: String,
    name: String,
    accepts_images: bool,
    thinking_levels: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredResources {
    extensions: Vec<DiscoveredResource>,
    skills: Vec<DiscoveredResource>,
    packages: Vec<DiscoveredPackage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredResource {
    id: String,
    name: String,
    path: String,
    enabled: bool,
    source: String,
    scope: DiscoveredScope,
    origin: DiscoveredOrigin,
    #[allow(dead_code)]
    #[serde(rename = "baseDir")]
    _base_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredPackage {
    id: String,
    name: String,
    source: String,
    scope: DiscoveredScope,
    filtered: bool,
    installed_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum DiscoveredScope {
    User,
    Project,
}

impl From<DiscoveredScope> for AgentResourceSource {
    fn from(scope: DiscoveredScope) -> Self {
        match scope {
            DiscoveredScope::User => Self::Piu,
            DiscoveredScope::Project => Self::Project,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum DiscoveredOrigin {
    Package,
    TopLevel,
}

impl From<DiscoveredOrigin> for AgentResourceOrigin {
    fn from(origin: DiscoveredOrigin) -> Self {
        match origin {
            DiscoveredOrigin::Package => Self::Package,
            DiscoveredOrigin::TopLevel => Self::TopLevel,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveredDiagnostic {
    resource_type: AgentEnvironmentDiagnosticResource,
    #[serde(rename = "type")]
    kind: AgentEnvironmentDiagnosticKind,
    message: String,
    path: Option<String>,
    source: Option<String>,
    scope: Option<DiscoveredScope>,
}

impl From<DiscoveredDiagnostic> for AgentEnvironmentDiagnostic {
    fn from(diagnostic: DiscoveredDiagnostic) -> Self {
        Self {
            resource: diagnostic.resource_type,
            kind: diagnostic.kind,
            message: diagnostic.message,
            path: diagnostic.path,
            source: diagnostic.source,
            source_scope: diagnostic.scope.map(Into::into),
        }
    }
}
