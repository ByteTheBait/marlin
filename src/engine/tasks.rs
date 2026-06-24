#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TaskStep {
    pub description: String,
    pub status: TaskStatus,
    pub tool_name: Option<String>,
}

impl TaskStep {
    pub fn goal(desc: impl Into<String>) -> Self {
        Self { description: desc.into(), status: TaskStatus::InProgress, tool_name: None }
    }

    pub fn tool_pending(name: &str, desc: impl Into<String>) -> Self {
        Self {
            description: desc.into(),
            status: TaskStatus::InProgress,
            tool_name: Some(name.to_string()),
        }
    }

    pub fn complete(mut self) -> Self {
        self.status = TaskStatus::Completed;
        self
    }

    pub fn fail(mut self) -> Self {
        self.status = TaskStatus::Failed;
        self
    }

    pub fn status_char(&self) -> char {
        match self.status {
            TaskStatus::Pending    => ' ',
            TaskStatus::InProgress => '>',
            TaskStatus::Completed  => 'x',
            TaskStatus::Failed     => '!',
        }
    }
}
