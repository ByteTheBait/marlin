#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    #[allow(dead_code)]
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TaskStep {
    pub description: String,
    pub status: TaskStatus,
    #[allow(dead_code)]
    pub tool_name: Option<String>,
    /// Set when this step ran concurrently with other steps sharing the same
    /// id (see `Engine::execute_tools`'s parallel-safe batching) — `None` for
    /// steps that ran sequentially/solo.
    pub parallel_group: Option<usize>,
}

impl TaskStep {
    #[allow(dead_code)]
    pub fn goal(desc: impl Into<String>) -> Self {
        Self {
            description: desc.into(),
            status: TaskStatus::InProgress,
            tool_name: None,
            parallel_group: None,
        }
    }

    /// Upfront plan step, shown Pending in the sidebar until the matching
    /// batch of tool calls completes (see `Engine::maybe_generate_plan`).
    pub fn planned(desc: impl Into<String>) -> Self {
        Self {
            description: desc.into(),
            status: TaskStatus::Pending,
            tool_name: None,
            parallel_group: None,
        }
    }

    pub fn tool_pending(
        name: &str,
        desc: impl Into<String>,
        parallel_group: Option<usize>,
    ) -> Self {
        Self {
            description: desc.into(),
            status: TaskStatus::InProgress,
            tool_name: Some(name.to_string()),
            parallel_group,
        }
    }

    #[allow(dead_code)]
    pub fn complete(mut self) -> Self {
        self.status = TaskStatus::Completed;
        self
    }

    #[allow(dead_code)]
    pub fn fail(mut self) -> Self {
        self.status = TaskStatus::Failed;
        self
    }

    #[allow(dead_code)]
    pub fn status_char(&self) -> char {
        match self.status {
            TaskStatus::Pending => ' ',
            TaskStatus::InProgress => '>',
            TaskStatus::Completed => 'x',
            TaskStatus::Failed => '!',
        }
    }
}
