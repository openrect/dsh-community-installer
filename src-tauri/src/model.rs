use serde::{Deserialize, Serialize};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DSH_VERSION: &str = env!("DSH_UPSTREAM_VERSION");
pub const DSH_DIST_TAGS: &str = env!("DSH_DIST_TAGS");
pub const NODE_VERSION: &str = env!("DSH_NODE_VERSION");
pub const PNPM_VERSION: &str = env!("DSH_PNPM_VERSION");
pub const HARNESS_URL: &str = "http://127.0.0.1:3080";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Locale {
    #[serde(rename = "zh-CN")]
    Chinese,
    #[default]
    #[serde(rename = "en-US")]
    English,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SetupPhase {
    #[default]
    NotInstalled,
    Preparing,
    Node,
    Dsh,
    Validating,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServicePhase {
    #[default]
    Stopped,
    Starting,
    Stopping,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateTarget {
    Controller,
    Dsh,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateDecision {
    Install,
    Later,
    Skip,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdatePhase {
    Checking,
    Available,
    Ready,
    Current,
    Failed,
    Installing,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub target: UpdateTarget,
    pub phase: UpdatePhase,
    pub version: Option<String>,
    pub progress: Option<f64>,
    pub resolved_items: Option<u64>,
    pub reused_items: Option<u64>,
    pub downloaded_items: Option<u64>,
    pub added_items: Option<u64>,
    pub total_items: Option<u64>,
    pub elapsed_seconds: Option<u64>,
    pub message_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub app_version: String,
    pub dsh_version: String,
    pub node_version: String,
    pub locale: Locale,
    pub edition: Edition,
    pub auto_check_dsh_updates: bool,
    pub setup_phase: SetupPhase,
    pub service_phase: ServicePhase,
    pub setup_complete: bool,
    pub progress: f64,
    pub message_key: String,
    pub update: Option<UpdateState>,
}

impl AppSnapshot {
    pub fn new(
        locale: Locale,
        edition: Edition,
        auto_check_dsh_updates: bool,
        setup_complete: bool,
    ) -> Self {
        Self {
            app_version: APP_VERSION.to_owned(),
            dsh_version: DSH_VERSION.to_owned(),
            node_version: NODE_VERSION.to_owned(),
            locale,
            edition,
            auto_check_dsh_updates,
            setup_phase: if setup_complete {
                SetupPhase::Complete
            } else {
                SetupPhase::NotInstalled
            },
            service_phase: ServicePhase::Stopped,
            setup_complete,
            progress: if setup_complete { 100.0 } else { 0.0 },
            message_key: if setup_complete {
                "complete"
            } else {
                "installCopy"
            }
            .to_owned(),
            update: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProgress {
    pub phase: SetupPhase,
    pub percent: f64,
    pub message_key: String,
    pub detail: Option<String>,
    pub resolved_items: Option<u64>,
    pub reused_items: Option<u64>,
    pub downloaded_items: Option<u64>,
    pub added_items: Option<u64>,
    pub total_items: Option<u64>,
    pub elapsed_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceState {
    pub phase: ServicePhase,
    pub message_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub timestamp: String,
    pub source: String,
    pub line: String,
}
