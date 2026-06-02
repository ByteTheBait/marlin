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

type GeminiProvider struct {
	apiKey string
}

func NewGeminiProvider(apiKey string) *GeminiProvider {
	return &GeminiProvider{apiKey: apiKey}
}

func (p *GeminiProvider) Name() string { return "gemini" }

func (p *GeminiProvider) Models() []string {
	return []string{
		"gemini-2.0-flash",
		"gemini-1.5-pro",
		"gemini-1.5-flash",
		"gemini-pro",
	}
}

type geminiContent struct {
	Role  string       `json:"role,omitempty"`
	Parts []geminiPart `json:"parts"`
}

type geminiPart struct {
	Text             string                 `json:"text,omitempty"`
	FunctionCall     *geminiFunctionCall    `json:"functionCall,omitempty"`
	FunctionResponse *geminiFunctionResp    `json:"functionResponse,omitempty"`
}

type geminiFunctionCall struct {
	Name string                 `json:"name"`
	Args map[string]interface{} `json:"args"`
}

type geminiFunctionResp struct {
	Name     string                 `json:"name"`
	Response map[string]interface{} `json:"response"`
}

type geminiRequest struct {
	Contents          []geminiContent  `json:"contents"`
	SystemInstruction *geminiContent   `json:"systemInstruction,omitempty"`
	Tools             []geminiToolSet  `json:"tools,omitempty"`
	GenerationConfig  struct {
		MaxOutputTokens int `json:"maxOutputTokens"`
	} `json:"generationConfig"`
}

type geminiToolSet struct {
	FunctionDeclarations []geminiFuncDecl `json:"functionDeclarations"`
}

type geminiFuncDecl struct {
	Name        string                 `json:"name"`
	Description string                 `json:"description"`
	Parameters  map[string]interface{} `json:"parameters,omitempty"`
}

func (p *GeminiProvider) Stream(ctx context.Context, model string, messages []Message, systemPrompt string, maxTokens int, tools []ToolDef) (<-chan StreamChunk, error) {
	if p.apiKey == "" {
		return nil, fmt.Errorf("gemini: no API key set — use /key gemini <your-key>")
	}

	contents := marshalGeminiContents(messages)

	reqBody := geminiRequest{Contents: contents}
	reqBody.GenerationConfig.MaxOutputTokens = maxTokens
	if systemPrompt != "" {
		reqBody.SystemInstruction = &geminiContent{
			Parts: []geminiPart{{Text: systemPrompt}},
		}
	}
	if len(tools) > 0 {
		reqBody.Tools = []geminiToolSet{{FunctionDeclarations: marshalGeminiTools(tools)}}
	}

	body, err := json.Marshal(reqBody)
	if err != nil {
		return nil, err
	}

	url := fmt.Sprintf(
		"https://generativelanguage.googleapis.com/v1beta/models/%s:streamGenerateContent?alt=sse&key=%s",
		model, p.apiKey,
	)
	req, err := http.NewRequestWithContext(ctx, "POST", url, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
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
			var errBody map[string]interface{}
			json.NewDecoder(resp.Body).Decode(&errBody)
			ch <- StreamChunk{Error: fmt.Errorf("gemini error %d: %v", resp.StatusCode, errBody)}
			return
		}

		var collectedCalls []ToolCall

		scanner := bufio.NewScanner(resp.Body)
		scanner.Buffer(make([]byte, 1024*1024), 1024*1024)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			data := strings.TrimPrefix(line, "data: ")

			var response struct {
				Candidates []struct {
					Content struct {
						Parts []struct {
							Text         string              `json:"text"`
							FunctionCall *geminiFunctionCall `json:"functionCall"`
						} `json:"parts"`
					} `json:"content"`
					FinishReason string `json:"finishReason"`
				} `json:"candidates"`
			}
			if err := json.Unmarshal([]byte(data), &response); err != nil {
				continue
			}

			for _, c := range response.Candidates {
				for _, part := range c.Content.Parts {
					if part.Text != "" {
						ch <- StreamChunk{Content: part.Text}
					}
					if part.FunctionCall != nil {
						argsJSON, _ := json.Marshal(part.FunctionCall.Args)
						collectedCalls = append(collectedCalls, ToolCall{
							ID:    part.FunctionCall.Name, // Gemini doesn't give IDs; use name
							Name:  part.FunctionCall.Name,
							Input: string(argsJSON),
						})
					}
				}
			}
		}

		if len(collectedCalls) > 0 {
			ch <- StreamChunk{Done: true, ToolCalls: collectedCalls}
		} else {
			ch <- StreamChunk{Done: true}
		}
	}()

	return ch, nil
}

func marshalGeminiContents(messages []Message) []geminiContent {
	out := make([]geminiContent, 0, len(messages))
	for _, m := range messages {
		switch {
		case m.Role == "assistant" && len(m.ToolCalls) > 0:
			var parts []geminiPart
			if m.Content != "" {
				parts = append(parts, geminiPart{Text: m.Content})
			}
			for _, tc := range m.ToolCalls {
				var args map[string]interface{}
				json.Unmarshal([]byte(tc.Input), &args)
				parts = append(parts, geminiPart{
					FunctionCall: &geminiFunctionCall{Name: tc.Name, Args: args},
				})
			}
			out = append(out, geminiContent{Role: "model", Parts: parts})

		case m.Role == "tool":
			out = append(out, geminiContent{
				Role: "user",
				Parts: []geminiPart{{
					FunctionResponse: &geminiFunctionResp{
						Name:     m.ToolUseID, // Gemini uses name as ID
						Response: map[string]interface{}{"content": m.Content},
					},
				}},
			})

		case m.Role == "assistant":
			out = append(out, geminiContent{Role: "model", Parts: []geminiPart{{Text: m.Content}}})

		case m.Role == "system":
			// Skip — handled via systemInstruction

		default:
			out = append(out, geminiContent{Role: "user", Parts: []geminiPart{{Text: m.Content}}})
		}
	}
	return out
}

func marshalGeminiTools(defs []ToolDef) []geminiFuncDecl {
	decls := make([]geminiFuncDecl, len(defs))
	for i, d := range defs {
		props := map[string]interface{}{}
		for k, p := range d.Properties {
			props[k] = map[string]string{"type": strings.ToUpper(p.Type), "description": p.Description}
		}
		params := map[string]interface{}{
			"type":       "OBJECT",
			"properties": props,
		}
		if len(d.Required) > 0 {
			params["required"] = d.Required
		}
		decls[i] = geminiFuncDecl{
			Name:        d.Name,
			Description: d.Description,
			Parameters:  params,
		}
	}
	return decls
}
