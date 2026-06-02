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

type OpenAICompatProvider struct {
	providerName string
	endpoint     string
	apiKey       string
	modelList    []string
}

func NewFireworksProvider(apiKey, endpoint string) *OpenAICompatProvider {
	if endpoint == "" {
		endpoint = "https://api.fireworks.ai/inference/v1"
	}
	return &OpenAICompatProvider{
		providerName: "fireworks",
		endpoint:     endpoint,
		apiKey:       apiKey,
		modelList: []string{
			"accounts/fireworks/models/llama-v3p1-70b-instruct",
			"accounts/fireworks/models/llama-v3p1-8b-instruct",
			"accounts/fireworks/models/mixtral-8x7b-instruct",
			"accounts/fireworks/models/deepseek-coder-v2-instruct",
		},
	}
}

func NewMoonshotProvider(apiKey, endpoint string) *OpenAICompatProvider {
	if endpoint == "" {
		endpoint = "https://api.moonshot.cn/v1"
	}
	return &OpenAICompatProvider{
		providerName: "moonshot",
		endpoint:     endpoint,
		apiKey:       apiKey,
		modelList:    []string{"moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"},
	}
}

func NewGroqProvider(apiKey, endpoint string) *OpenAICompatProvider {
	if endpoint == "" {
		endpoint = "https://api.groq.com/openai/v1"
	}
	return &OpenAICompatProvider{
		providerName: "groq",
		endpoint:     endpoint,
		apiKey:       apiKey,
		modelList: []string{
			"llama-3.3-70b-versatile",
			"llama-3.1-8b-instant",
			"llama3-70b-8192",
			"llama3-8b-8192",
			"mixtral-8x7b-32768",
			"gemma2-9b-it",
		},
	}
}

func NewCustomProvider(apiKey, endpoint string) *OpenAICompatProvider {
	if endpoint == "" {
		endpoint = "http://localhost:8080/v1"
	}
	return &OpenAICompatProvider{
		providerName: "custom",
		endpoint:     endpoint,
		apiKey:       apiKey,
		modelList:    []string{"default"},
	}
}

func (p *OpenAICompatProvider) Name() string     { return p.providerName }
func (p *OpenAICompatProvider) Models() []string { return p.modelList }

func (p *OpenAICompatProvider) Stream(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error) {
	if p.apiKey == "" && p.providerName != "custom" {
		return nil, fmt.Errorf("%s: no API key set — use /key %s <your-key>", p.providerName, p.providerName)
	}

	oaiMessages := marshalOpenAIMessages(messages, systemPrompt)
	oaiTools := marshalOpenAITools(tools)

	reqBody := map[string]interface{}{
		"model":      model,
		"messages":   oaiMessages,
		"stream":     true,
		"max_tokens": maxTokens,
	}
	if len(oaiTools) > 0 {
		reqBody["tools"] = oaiTools
	}

	body, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", p.endpoint+"/chat/completions", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("content-type", "application/json")
	if p.apiKey != "" {
		req.Header.Set("authorization", "Bearer "+p.apiKey)
	}

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
			var errBody map[string]interface{}
			json.NewDecoder(resp.Body).Decode(&errBody)
			ch <- StreamChunk{Error: fmt.Errorf("%s error %d: %v", p.providerName, resp.StatusCode, errBody)}
			return
		}

		rl := ParseRateLimitState(resp)

		// Accumulate tool calls by their index (OpenAI streams them in fragments).
		type accToolCall struct {
			id   string
			name string
			args strings.Builder
		}
		acc := map[int]*accToolCall{}

		scanner := bufio.NewScanner(resp.Body)
		scanner.Buffer(make([]byte, 1024*1024), 1024*1024)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			data := strings.TrimPrefix(line, "data: ")
			if data == "[DONE]" {
				break
			}

			var event struct {
				Choices []struct {
					Delta struct {
						Content   *string `json:"content"`
						ToolCalls []struct {
							Index    int    `json:"index"`
							ID       string `json:"id"`
							Function struct {
								Name      string `json:"name"`
								Arguments string `json:"arguments"`
							} `json:"function"`
						} `json:"tool_calls"`
					} `json:"delta"`
					FinishReason *string `json:"finish_reason"`
				} `json:"choices"`
			}
			if err := json.Unmarshal([]byte(data), &event); err != nil {
				continue
			}

			for _, choice := range event.Choices {
				// Text content
				if choice.Delta.Content != nil && *choice.Delta.Content != "" {
					ch <- StreamChunk{Content: *choice.Delta.Content}
				}
				// Tool call fragments
				for _, tc := range choice.Delta.ToolCalls {
					if _, ok := acc[tc.Index]; !ok {
						acc[tc.Index] = &accToolCall{}
					}
					a := acc[tc.Index]
					if tc.ID != "" {
						a.id = tc.ID
					}
					if tc.Function.Name != "" {
						a.name = tc.Function.Name
					}
					a.args.WriteString(tc.Function.Arguments)
				}
			}
		}

		if len(acc) > 0 {
			calls := make([]ToolCall, 0, len(acc))
			for i := 0; i < len(acc); i++ {
				a, ok := acc[i]
				if !ok {
					continue
				}
				inp := a.args.String()
				if inp == "" {
					inp = "{}"
				}
				calls = append(calls, ToolCall{ID: a.id, Name: a.name, Input: inp})
			}
			ch <- StreamChunk{Done: true, ToolCalls: calls, RateLimit: rl}
		} else {
			ch <- StreamChunk{Done: true, RateLimit: rl}
		}
	}()

	return ch, nil
}

// marshalOpenAIMessages converts generic messages to the OpenAI wire format.
func marshalOpenAIMessages(messages []Message, systemPrompt string) []map[string]interface{} {
	out := []map[string]interface{}{}
	if systemPrompt != "" {
		out = append(out, map[string]interface{}{"role": "system", "content": systemPrompt})
	}
	for _, m := range messages {
		switch {
		case m.Role == "assistant" && len(m.ToolCalls) > 0:
			calls := make([]map[string]interface{}, len(m.ToolCalls))
			for i, tc := range m.ToolCalls {
				calls[i] = map[string]interface{}{
					"id":   tc.ID,
					"type": "function",
					"function": map[string]string{
						"name":      tc.Name,
						"arguments": tc.Input,
					},
				}
			}
			msg := map[string]interface{}{
				"role":       "assistant",
				"tool_calls": calls,
			}
			if m.Content != "" {
				msg["content"] = m.Content
			}
			out = append(out, msg)

		case m.Role == "tool":
			out = append(out, map[string]interface{}{
				"role":         "tool",
				"tool_call_id": m.ToolCallID,
				"content":      m.Content,
			})

		default:
			out = append(out, map[string]interface{}{"role": m.Role, "content": m.Content})
		}
	}
	return out
}

func marshalOpenAITools(defs []ToolDef) []map[string]interface{} {
	if len(defs) == 0 {
		return nil
	}
	tools := make([]map[string]interface{}, len(defs))
	for i, d := range defs {
		props := map[string]interface{}{}
		for k, p := range d.Properties {
			props[k] = map[string]string{"type": p.Type, "description": p.Description}
		}
		tools[i] = map[string]interface{}{
			"type": "function",
			"function": map[string]interface{}{
				"name":        d.Name,
				"description": d.Description,
				"parameters": map[string]interface{}{
					"type":       "object",
					"properties": props,
					"required":   d.Required,
				},
			},
		}
	}
	return tools
}
