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

type OllamaProvider struct {
	endpoint string
}

func NewOllamaProvider(endpoint string) *OllamaProvider {
	if endpoint == "" {
		endpoint = "http://localhost:11434"
	}
	return &OllamaProvider{endpoint: endpoint}
}

func (p *OllamaProvider) Name() string { return "ollama" }

func (p *OllamaProvider) Models() []string {
	return []string{"llama3", "llama3:70b", "codellama", "mistral", "mixtral", "phi3", "gemma2", "qwen2"}
}

func (p *OllamaProvider) Stream(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error) {
	// Ollama >=0.3 supports the OpenAI-compatible /v1/chat/completions endpoint
	// which handles tools. Fall back to /api/chat (no tools) for older versions.
	if len(tools) > 0 {
		return p.streamOpenAI(ctx, model, messages, systemPrompt, maxTokens, tools)
	}
	return p.streamNative(ctx, model, messages, systemPrompt, maxTokens)
}

func (p *OllamaProvider) streamOpenAI(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error) {
	oaiMessages := marshalOpenAIMessages(messages, systemPrompt)
	oaiTools := marshalOpenAITools(tools)

	reqBody := map[string]interface{}{
		"model":    model,
		"messages": oaiMessages,
		"stream":   true,
		"options":  map[string]int{"num_predict": maxTokens},
	}
	if len(oaiTools) > 0 {
		reqBody["tools"] = oaiTools
	}

	body, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", p.endpoint+"/v1/chat/completions", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("content-type", "application/json")

	ch := make(chan StreamChunk, 64)
	go func() {
		defer close(ch)

		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			// Fall back to native endpoint on connection error to /v1
			ch <- StreamChunk{Error: fmt.Errorf("ollama: %w (is Ollama running? try: ollama serve)", err)}
			return
		}
		defer resp.Body.Close()

		if resp.StatusCode == 429 {
			ch <- StreamChunk{RetryAfter: retryAfterSeconds(resp, 30)}
			return
		}
		if resp.StatusCode != 200 {
			ch <- StreamChunk{Error: fmt.Errorf("ollama /v1 returned %d — try upgrading Ollama for tool support", resp.StatusCode)}
			return
		}

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
				} `json:"choices"`
			}
			if err := json.Unmarshal([]byte(data), &event); err != nil {
				continue
			}
			for _, choice := range event.Choices {
				if choice.Delta.Content != nil && *choice.Delta.Content != "" {
					ch <- StreamChunk{Content: *choice.Delta.Content}
				}
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
			ch <- StreamChunk{Done: true, ToolCalls: calls}
		} else {
			ch <- StreamChunk{Done: true}
		}
	}()
	return ch, nil
}

func (p *OllamaProvider) streamNative(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int) (<-chan StreamChunk, error) {
	msgs := messages
	if systemPrompt != "" {
		msgs = append([]Message{{Role: "system", Content: systemPrompt}}, messages...)
	}

	type ollamaMsg struct {
		Role    string `json:"role"`
		Content string `json:"content"`
	}
	omsgs := make([]ollamaMsg, 0, len(msgs))
	for _, m := range msgs {
		if m.Role == "tool" {
			omsgs = append(omsgs, ollamaMsg{Role: "user", Content: "[tool result] " + m.Content})
		} else if m.Role != "tool" {
			omsgs = append(omsgs, ollamaMsg{Role: m.Role, Content: m.Content})
		}
	}

	reqBody := map[string]interface{}{
		"model":    model,
		"messages": omsgs,
		"stream":   true,
		"options":  map[string]int{"num_predict": maxTokens},
	}

	body, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequestWithContext(ctx, "POST", p.endpoint+"/api/chat", bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Header.Set("content-type", "application/json")

	ch := make(chan StreamChunk, 64)
	go func() {
		defer close(ch)

		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			ch <- StreamChunk{Error: fmt.Errorf("ollama: %w (is Ollama running? try: ollama serve)", err)}
			return
		}
		defer resp.Body.Close()

		scanner := bufio.NewScanner(resp.Body)
		scanner.Buffer(make([]byte, 1024*1024), 1024*1024)
		for scanner.Scan() {
			var event struct {
				Message struct {
					Content string `json:"content"`
				} `json:"message"`
				Done bool `json:"done"`
			}
			if err := json.Unmarshal(scanner.Bytes(), &event); err != nil {
				continue
			}
			if event.Message.Content != "" {
				ch <- StreamChunk{Content: event.Message.Content}
			}
			if event.Done {
				break
			}
		}
		ch <- StreamChunk{Done: true}
	}()

	return ch, nil
}
