package providers

import (
	"net/http"
	"strconv"
)

// retryAfterSeconds reads the Retry-After header from a 429 response.
// Returns defaultSecs if the header is absent or unparseable.
func retryAfterSeconds(resp *http.Response, defaultSecs int) int {
	if ra := resp.Header.Get("retry-after"); ra != "" {
		if n, err := strconv.Atoi(ra); err == nil && n > 0 {
			return n
		}
	}
	// x-ratelimit-reset-requests is a Unix timestamp on some APIs; skip for now.
	return defaultSecs
}
