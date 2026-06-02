package providers

import "context"

// Message is a chat history entry. Role is one of: user, assistant, tool.
// When Role=="assistant" and ToolCalls is non-empty, the assistant used tools.
// When Role=="tool", Content is the tool result; ToolUseID/ToolCallID tie it back.
type Message struct {
	Role       string        `json:"role"`
	Content    string        `json:"content"`
	ToolCalls  []ToolCallMsg `json:"tool_calls,omitempty"`
	ToolUseID  string        `json:"tool_use_id,omitempty"`  // Claude tool result
	ToolCallID string        `json:"tool_call_id,omitempty"` // OpenAI tool result
	IsError    bool          `json:"is_error,omitempty"`
}

// ToolCallMsg records one tool invocation inside an assistant message.
type ToolCallMsg struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Input string `json:"input"` // JSON-encoded input object
}

// ToolCall is the parsed tool call received from a streaming response.
type ToolCall struct {
	ID    string
	Name  string
	Input string // complete JSON input object
}

// StreamChunk is one unit of output from a streaming provider response.
// When Done is true and ToolCalls is non-empty, the model made tool calls.
// When RetryAfter > 0 the request was rate-limited; retry after that duration.
type StreamChunk struct {
	Content    string
	Done       bool
	Error      error
	ToolCalls  []ToolCall
	RetryAfter int // seconds to wait before retrying (rate limit); 0 = not rate-limited
}

// ToolDef is the provider-facing schema for a single tool.
type ToolDef struct {
	Name        string
	Description string
	Properties  map[string]ToolProp
	Required    []string
}

// ToolProp describes one input parameter.
type ToolProp struct {
	Type        string
	Description string
}

// Provider is the interface every AI backend must satisfy.
type Provider interface {
	Name() string
	Models() []string
	Stream(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error)
}
