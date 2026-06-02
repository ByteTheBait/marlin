package providers

import (
	"fmt"
	"sort"

	"marlin/internal/config"
)

type Registry struct {
	providers map[string]Provider
}

func NewRegistry(cfg *config.Config) *Registry {
	r := &Registry{providers: make(map[string]Provider)}

	get := func(name string) config.ProviderConfig {
		if c, ok := cfg.Providers[name]; ok {
			return c
		}
		return config.ProviderConfig{}
	}

	r.providers["claude"] = NewClaudeProvider(get("claude").APIKey)
	r.providers["gemini"] = NewGeminiProvider(get("gemini").APIKey)
	r.providers["ollama"] = NewOllamaProvider(get("ollama").Endpoint)
	r.providers["fireworks"] = NewFireworksProvider(get("fireworks").APIKey, get("fireworks").Endpoint)
	r.providers["moonshot"] = NewMoonshotProvider(get("moonshot").APIKey, get("moonshot").Endpoint)
	r.providers["groq"] = NewGroqProvider(get("groq").APIKey, get("groq").Endpoint)
	r.providers["custom"] = NewCustomProvider(get("custom").APIKey, get("custom").Endpoint)

	return r
}

func (r *Registry) Get(name string) (Provider, error) {
	p, ok := r.providers[name]
	if !ok {
		return nil, fmt.Errorf("unknown provider %q — available: claude, gemini, ollama, fireworks, moonshot, groq, custom", name)
	}
	return p, nil
}

func (r *Registry) Names() []string {
	names := make([]string, 0, len(r.providers))
	for name := range r.providers {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}
