package config

import (
	"encoding/json"
	"os"
	"path/filepath"
)

type ProviderConfig struct {
	APIKey   string `json:"api_key,omitempty"`
	Endpoint string `json:"endpoint,omitempty"`
	Model    string `json:"model,omitempty"`
}

type Config struct {
	ActiveProvider  string                    `json:"active_provider"`
	ActiveModel     string                    `json:"active_model"`
	Providers       map[string]ProviderConfig `json:"providers"`
	AllowedCommands []string                  `json:"allowed_commands"`
	SystemPrompt    string                    `json:"system_prompt,omitempty"`
	MaxTokens       int                       `json:"max_tokens"`
	WorkDir         string                    `json:"work_dir,omitempty"`
}

func Dir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(home, ".marlin")
	return dir, os.MkdirAll(dir, 0755)
}

func Load() (*Config, error) {
	dir, err := Dir()
	if err != nil {
		return nil, err
	}
	path := filepath.Join(dir, "config.json")

	cfg := defaults()

	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return cfg, cfg.Save()
	}
	if err != nil {
		return nil, err
	}

	if err := json.Unmarshal(data, cfg); err != nil {
		return nil, err
	}

	// Fill any missing provider defaults
	for k, v := range defaults().Providers {
		if _, ok := cfg.Providers[k]; !ok {
			cfg.Providers[k] = v
		}
	}

	return cfg, nil
}

func (c *Config) Save() error {
	dir, err := Dir()
	if err != nil {
		return err
	}
	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, "config.json"), data, 0600)
}

func (c *Config) SetKey(provider, key string) {
	p := c.Providers[provider]
	p.APIKey = key
	c.Providers[provider] = p
}

func (c *Config) SetEndpoint(provider, endpoint string) {
	p := c.Providers[provider]
	p.Endpoint = endpoint
	c.Providers[provider] = p
}

func defaults() *Config {
	wd, _ := os.Getwd()
	return &Config{
		ActiveProvider: "claude",
		ActiveModel:    "claude-sonnet-4-5",
		Providers: map[string]ProviderConfig{
			"claude":    {Model: "claude-sonnet-4-5"},
			"gemini":    {Model: "gemini-1.5-pro"},
			"ollama":    {Endpoint: "http://localhost:11434", Model: "llama3"},
			"fireworks": {Endpoint: "https://api.fireworks.ai/inference/v1", Model: "accounts/fireworks/models/llama-v3-70b-instruct"},
			"moonshot":  {Endpoint: "https://api.moonshot.cn/v1", Model: "moonshot-v1-8k"},
			"groq":      {Endpoint: "https://api.groq.com/openai/v1", Model: "llama-3.3-70b-versatile"},
			"custom":    {Endpoint: "http://localhost:8080/v1", Model: "default"},
		},
		AllowedCommands: []string{},
		MaxTokens:       4096,
		WorkDir:         wd,
	}
}
