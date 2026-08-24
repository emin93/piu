use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, State};
use ts_rs::TS;

use crate::application::ApplicationCore;

pub const HOST_ROUND_TRIP_EVENT: &str = "host://round-trip-completed";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct HostRoundTripRequest {
    pub correlation_id: String,
    #[ts(type = "number")]
    pub sent_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/generated/")]
pub struct HostRoundTripResponse {
    pub correlation_id: String,
    #[ts(type = "number")]
    pub sent_at_ms: u64,
    #[ts(type = "number")]
    pub received_at_ms: u64,
    pub schema_version: u32,
}

#[tauri::command]
pub async fn host_round_trip<R: Runtime>(
    app: AppHandle<R>,
    core: State<'_, ApplicationCore>,
    request: HostRoundTripRequest,
) -> Result<HostRoundTripResponse, String> {
    let received_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| "system time does not fit in a 64-bit millisecond value")?;
    let project_inbox = core.project_inbox();
    let schema_version =
        tauri::async_runtime::spawn_blocking(move || project_inbox.schema_version())
            .await
            .map_err(|_| "application storage worker stopped unexpectedly".to_owned())?
            .map_err(|error| error.to_string())?;
    let response = HostRoundTripResponse {
        correlation_id: request.correlation_id,
        sent_at_ms: request.sent_at_ms,
        received_at_ms,
        schema_version,
    };
    app.emit(HOST_ROUND_TRIP_EVENT, &response)
        .map_err(|error| format!("could not emit host round-trip event: {error}"))?;
    tracing::info!(target: "piu::startup", "piu_shell_ready");
    Ok(response)
}
