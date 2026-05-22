use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveStrategy {
    SafeCopyDelete,
    RobocopyMove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Planned,
    Copying,
    Copied,
    DeletingSource,
    Linking,
    Completed,
    Cancelled,
    Failed,
    RollingBack,
    RolledBack,
}

impl OperationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationStatus::Planned => "planned",
            OperationStatus::Copying => "copying",
            OperationStatus::Copied => "copied",
            OperationStatus::DeletingSource => "deleting_source",
            OperationStatus::Linking => "linking",
            OperationStatus::Completed => "completed",
            OperationStatus::Cancelled => "cancelled",
            OperationStatus::Failed => "failed",
            OperationStatus::RollingBack => "rolling_back",
            OperationStatus::RolledBack => "rolled_back",
        }
    }
}

impl TryFrom<&str> for OperationStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "planned" => Ok(OperationStatus::Planned),
            "copying" => Ok(OperationStatus::Copying),
            "copied" => Ok(OperationStatus::Copied),
            "deleting_source" => Ok(OperationStatus::DeletingSource),
            "linking" => Ok(OperationStatus::Linking),
            "completed" => Ok(OperationStatus::Completed),
            "cancelled" => Ok(OperationStatus::Cancelled),
            "failed" => Ok(OperationStatus::Failed),
            "rolling_back" => Ok(OperationStatus::RollingBack),
            "rolled_back" => Ok(OperationStatus::RolledBack),
            other => Err(format!("unknown operation status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRequest {
    pub source_path: String,
    pub destination_parent: String,
    pub strategy: MoveStrategy,
    pub skip_lock_check: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    pub path: String,
    pub pid: u32,
    pub process_name: String,
    pub application_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovePreview {
    pub source_path: String,
    pub destination_path: String,
    pub item_kind: ItemKind,
    pub locks: Vec<FileLock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    pub current: u64,
    pub total: u64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationSnapshot {
    pub id: String,
    pub source_path: String,
    pub destination_path: String,
    pub item_kind: ItemKind,
    pub strategy: MoveStrategy,
    pub status: OperationStatus,
    pub created_at: String,
    pub updated_at: String,
    pub log_path: String,
    pub error_message: Option<String>,
    pub progress_current: Option<u64>,
    pub progress_total: Option<u64>,
    pub progress_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperAction {
    Move,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperInvocation {
    pub action: HelperAction,
    pub operation: OperationSnapshot,
    pub cancel_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRead {
    pub lines: Vec<String>,
    pub next_offset: u64,
}
