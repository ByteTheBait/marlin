package providers

import (
	"net/http"
	"strconv"
	"time"
)

// retryAfterSeconds returns how many seconds to wait before retrying a 429.
//
// Resolution order:
//  1. Retry-After header (seconds integer or HTTP date)
//  2. Provider-specific reset headers (ISO 8601, Unix timestamp, or seconds)
//     — we take the maximum so we wait until every limit has cleared.
//  3. defaultSecs if nothing parseable is found.
func retryAfterSeconds(resp *http.Response, defaultSecs int) int {
	// Retry-After is the authoritative HTTP header.
	if ra := resp.Header.Get("retry-after"); ra != "" {
		if secs := parseWaitSeconds(ra); secs > 0 {
			return secs
		}
	}

	// Vendor-specific reset headers used by Anthropic, OpenAI-compat, Groq, etc.
	// Take the maximum — wait until all limits have cleared.
	resetHeaders := []string{
		"x-ratelimit-reset-requests",
		"x-ratelimit-reset-tokens",
		"x-ratelimit-reset",
		"anthropic-ratelimit-requests-reset",
		"anthropic-ratelimit-tokens-reset",
		"ratelimit-reset",
	}
	best := 0
	for _, h := range resetHeaders {
		if val := resp.Header.Get(h); val != "" {
			if secs := parseWaitSeconds(val); secs > best {
				best = secs
			}
		}
	}
	if best > 0 {
		return best
	}

	return defaultSecs
}

// parseWaitSeconds converts a header value to a number of seconds from now.
// Handles:
//   - Plain integer seconds ("120")
//   - Unix timestamp as integer ("1735000000")
//   - ISO 8601 / RFC 3339 ("2024-01-15T10:30:00Z")
//   - HTTP date ("Mon, 15 Jan 2024 10:30:00 GMT")
//   - Go duration strings ("5h30m")
func parseWaitSeconds(val string) int {
	// Plain integer — could be seconds or a Unix timestamp.
	if n, err := strconv.ParseInt(val, 10, 64); err == nil {
		if n > 1_000_000_000 {
			// Looks like a Unix timestamp.
			if secs := int(time.Until(time.Unix(n, 0)).Seconds()); secs > 0 {
				return secs
			}
			return 0
		}
		if n > 0 {
			return int(n)
		}
		return 0
	}

	// Time-based formats — compute seconds until that moment.
	timeFormats := []string{
		time.RFC3339Nano,
		time.RFC3339,
		"2006-01-02T15:04:05Z",
		http.TimeFormat, // "Mon, 02 Jan 2006 15:04:05 GMT"
		time.RFC850,
		time.ANSIC,
	}
	for _, layout := range timeFormats {
		if t, err := time.Parse(layout, val); err == nil {
			if secs := int(time.Until(t).Seconds()); secs > 0 {
				return secs
			}
			return 0
		}
	}

	// Go duration string ("5h", "30m", "1h30m").
	if d, err := time.ParseDuration(val); err == nil && d > 0 {
		return int(d.Seconds())
	}

	return 0
}
