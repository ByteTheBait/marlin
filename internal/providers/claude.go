package providers

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
)

type ClaudeProvider struct {
	apiKey string
}

func NewClaudeProvider(apiKey string) *ClaudeProvider {
	return &ClaudeProvider{apiKey: apiKey}
}

func (p *ClaudeProvider) Name() string { return "claude" }

func (p *ClaudeProvider) Models() []string {
	return []string{
		"claude-opus-4-5",
		"claude-sonnet-4-5",
		"claude-haiku-4-5-20251001",
		"claude-3-5-sonnet-20241022",
		"claude-3-5-haiku-20241022",
	}
}

// claudeMessage is the wire format for the Claude messages API.
type claudeMessage struct {
	Role    string      `json:"role"`
	Content interface{} `json:"content"` // string | []claudeBlock
}

type claudeBlock struct {
	Type      string      `json:"type"`
	Text      string      `json:"text,omitempty"`
	ID        string      `json:"id,omitempty"`
	Name      string      `json:"name,omitempty"`
	Input     interface{} `json:"input,omitempty"`
	ToolUseID string      `json:"tool_use_id,omitempty"`
	Content   interface{} `json:"content,omitempty"`
	IsError   bool        `json:"is_error,omitempty"`
}

type claudeTool struct {
	Name        string                 `json:"name"`
	Description string                 `json:"description"`
	InputSchema map[string]interface{} `json:"input_schema"`
}

type claudeRequest struct {
	Model     string          `json:"model"`
	Messages  []claudeMessage `json:"messages"`
	System    string          `json:"system,omitempty"`
	MaxTokens int             `json:"max_tokens"`
	Stream    bool            `json:"stream"`
	Tools     []claudeTool    `json:"tools,omitempty"`
}

func (p *ClaudeProvider) Stream(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error) {
	if p.apiKey == "" {
		return nil, fmt.Errorf("claude: no API key set — use /key claude <your-key>")
	}

	claudeMessages := marshalClaudeMessages(messages)
	claudeTools := marshalClaudeTools(tools)

	body, err := json.Marshal(claudeRequest{
		Model:     model,
		Messages:  claudeMessages,
		System:    systemPrompt,
		MaxTokens: maxTokens,
		Stream:    true,
		Tools:     claudeTools,
	})
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", "https://api.anthropic.com/v1/messages", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("x-api-key", p.apiKey)
	req.Header.Set("anthropic-version", "2023-06-01")
	req.Header.Set("content-type", "application/json")

	ch := make(chan StreamChunk, 64)
	go func() {
		defer close(ch)

		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			ch <- StreamChunk{Error: err}
			return
		}
		defer resp.Body.Close()

		if resp.StatusCode == 429 {
			ch <- StreamChunk{RetryAfter: retryAfterSeconds(resp, 60)}
			return
		}
		if resp.StatusCode != 200 {
			var errBody struct {
				Error struct {
					Message string `json:"message"`
				} `json:"error"`
			}
			json.NewDecoder(resp.Body).Decode(&errBody)
			ch <- StreamChunk{Error: fmt.Errorf("claude error %d: %s", resp.StatusCode, errBody.Error.Message)}
			return
		}

		// Track in-progress tool calls being accumulated from the stream.
		type pendingTool struct {
			id    string
			name  string
			input strings.Builder
		}
		var pending []*pendingTool
		var currentTool *pendingTool

		scanner := bufio.NewScanner(resp.Body)
		scanner.Buffer(make([]byte, 1024*1024), 1024*1024)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			data := strings.TrimPrefix(line, "data: ")

			var event map[string]json.RawMessage
			if err := json.Unmarshal([]byte(data), &event); err != nil {
				continue
			}

			var evType string
			json.Unmarshal(event["type"], &evType)

			switch evType {
			case "content_block_start":
				var block struct {
					Index        int `json:"index"`
					ContentBlock struct {
						Type string `json:"type"`
						ID   string `json:"id"`
						Name string `json:"name"`
					} `json:"content_block"`
				}
				if json.Unmarshal([]byte(data), &block) == nil && block.ContentBlock.Type == "tool_use" {
					currentTool = &pendingTool{
						id:   block.ContentBlock.ID,
						name: block.ContentBlock.Name,
					}
					pending = append(pending, currentTool)
				} else {
					currentTool = nil
				}

			case "content_block_delta":
				var delta struct {
					Delta struct {
						Type     string `json:"type"`
						Text     string `json:"text"`
						PartJSON string `json:"partial_json"`
					} `json:"delta"`
				}
				if json.Unmarshal([]byte(data), &delta) == nil {
					switch delta.Delta.Type {
					case "text_delta":
						if delta.Delta.Text != "" {
							ch <- StreamChunk{Content: delta.Delta.Text}
						}
					case "input_json_delta":
						if currentTool != nil {
							currentTool.input.WriteString(delta.Delta.PartJSON)
						}
					}
				}

			case "content_block_stop":
				currentTool = nil

			case "message_stop":
				// Collect completed tool calls
				if len(pending) > 0 {
					calls := make([]ToolCall, len(pending))
					for i, pt := range pending {
						inp := pt.input.String()
						if inp == "" {
							inp = "{}"
						}
						calls[i] = ToolCall{ID: pt.id, Name: pt.name, Input: inp}
					}
					ch <- StreamChunk{Done: true, ToolCalls: calls}
				} else {
					ch <- StreamChunk{Done: true}
				}
				return
			}
		}
		ch <- StreamChunk{Done: true}
	}()

	return ch, nil
}

// marshalClaudeMessages converts the generic message history to Claude wire format.
func marshalClaudeMessages(msgs []Message) []claudeMessage {
	out := make([]claudeMessage, 0, len(msgs))
	for _, m := range msgs {
		switch {
		case m.Role == "assistant" && len(m.ToolCalls) > 0:
			var content []claudeBlock
			if m.Content != "" {
				content = append(content, claudeBlock{Type: "text", Text: m.Content})
			}
			for _, tc := range m.ToolCalls {
				var input interface{}
				json.Unmarshal([]byte(tc.Input), &input)
				if input == nil {
					input = map[string]interface{}{}
				}
				content = append(content, claudeBlock{
					Type:  "tool_use",
					ID:    tc.ID,
					Name:  tc.Name,
					Input: input,
				})
			}
			out = append(out, claudeMessage{Role: "assistant", Content: content})

		case m.Role == "tool":
			// Tool results go back as user messages with tool_result blocks.
			block := claudeBlock{
				Type:      "tool_result",
				ToolUseID: m.ToolUseID,
				Content:   m.Content,
				IsError:   m.IsError,
			}
			out = append(out, claudeMessage{Role: "user", Content: []claudeBlock{block}})

		default:
			out = append(out, claudeMessage{Role: m.Role, Content: m.Content})
		}
	}
	return out
}

func marshalClaudeTools(defs []ToolDef) []claudeTool {
	if len(defs) == 0 {
		return nil
	}
	tools := make([]claudeTool, len(defs))
	for i, d := range defs {
		props := map[string]interface{}{}
		for k, p := range d.Properties {
			props[k] = map[string]string{"type": p.Type, "description": p.Description}
		}
		tools[i] = claudeTool{
			Name:        d.Name,
			Description: d.Description,
			InputSchema: map[string]interface{}{
				"type":       "object",
				"properties": props,
				"required":   d.Required,
			},
		}
	}
	return tools
}
