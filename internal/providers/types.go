package providers

import (
	"context"
	"time"
)

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

// RateLimitState carries the budget info parsed from a successful response's
// headers. Used for proactive pausing before the next request would exceed limits.
type RateLimitState struct {
	RemainingRequests int       // -1 = unknown
	RemainingTokens   int       // -1 = unknown
	ResetRequestsAt   time.Time // zero = unknown
	ResetTokensAt     time.Time // zero = unknown
}

// StreamChunk is one unit of output from a streaming provider response.
// When Done is true and ToolCalls is non-empty, the model made tool calls.
// When RetryAfter > 0 the request was rate-limited; retry after that duration.
// RateLimit is populated on the Done chunk when the provider returned budget headers.
type StreamChunk struct {
	Content    string
	Done       bool
	Error      error
	ToolCalls  []ToolCall
	RetryAfter int            // seconds to wait before retrying (rate limit); 0 = not rate-limited
	RateLimit  *RateLimitState // nil if provider sent no budget headers
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
