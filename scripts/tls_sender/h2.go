package main

import (
	"bytes"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"time"

	utls "github.com/refraction-networking/utls"
	"golang.org/x/net/http2"
)

// h2Live sends the single authorized request over HTTP/2 through the uTLS
// fingerprint. Framer's edge negotiates h2, so reading the verdict needs h2.
// Prints status and a redacted head of the body (tokens/credits are numbers
// or short strings; the body is the model's reply, safe to show).
func h2Live() {
	specFile := os.Getenv("LIVE_REQUEST_FILE")
	if specFile == "" {
		fmt.Println("h2-skip: LIVE_REQUEST_FILE unset")
		return
	}
	data, err := os.ReadFile(specFile)
	if err != nil {
		fmt.Println("h2-read-error:", err)
		return
	}
	var req struct {
		Addr    string            `json:"addr"`
		Host    string            `json:"host"`
		Path    string            `json:"path"`
		Headers map[string]string `json:"headers"`
		Body    string            `json:"body"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		fmt.Println("h2-parse-error:", err)
		return
	}
	spec := exactSpec()
	dial, err := net.DialTimeout("tcp", req.Addr, 8*time.Second)
	if err != nil {
		fmt.Println("h2-dial-error:", err)
		return
	}
	cfg := &utls.Config{ServerName: req.Host, InsecureSkipVerify: true, NextProtos: []string{"h2"}}
	conn := utls.UClient(dial, cfg, utls.HelloCustom)
	if err := conn.ApplyPreset(&spec); err != nil {
		fmt.Println("h2-apply-error:", err)
		return
	}
	if err := conn.Handshake(); err != nil {
		fmt.Println("h2-handshake-error:", err)
		return
	}
	if got := conn.ConnectionState().NegotiatedProtocol; got != "h2" {
		fmt.Println("h2-abort: negotiated", got)
		conn.Close()
		return
	}

	tr := &http2.Transport{
		// Disable Go's automatic "Accept-Encoding: gzip" so the request can carry
		// the exact accept-encoding value a real Chrome sends
		// ("gzip, deflate, br, zstd"). Without this the wire shows Go's gzip-only
		// value, which is a readable non-browser signal.
		DisableCompression: true,
		DialTLSContext: nil,
		DialTLS: func(network, addr string, _ *tls.Config) (net.Conn, error) {
			return conn, nil
		},
	}
	url := "https://" + req.Host + req.Path
	httpReq, err := http.NewRequest("POST", url, bytes.NewReader([]byte(req.Body)))
	if err != nil {
		fmt.Println("h2-newreq-error:", err)
		return
	}
	for k, v := range req.Headers {
		httpReq.Header.Set(k, v)
	}
	_ = time.Now()
	resp, err := tr.RoundTrip(httpReq)
	if err != nil {
		fmt.Println("h2-roundtrip-error:", err)
		return
	}
	defer resp.Body.Close()
	// Read the raw body without Go's transparent gzip so the refusal reason is
	// readable (Framer's 403s are short JSON/text; some edges gzip them).
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
	usage := resp.Header.Get("X-Framer-AI-Usage")
	fmt.Printf("h2-status=%d x-framer-ai-usage=%q\n", resp.StatusCode, usage)
	fmt.Printf("h2-content-type=%q content-encoding=%q\n", resp.Header.Get("Content-Type"), resp.Header.Get("Content-Encoding"))
	fmt.Printf("h2-body=%s\n", redactBody(body))
	// Print all response headers for the refusal diagnosis (values may name the
	// rule: velocity / verification / policy / fingerprint).
	for k, vs := range resp.Header {
		for _, v := range vs {
			fmt.Printf("h2-rhdr %s: %s\n", k, v)
		}
	}
	conn.Close()
}

// redactBody scrubs anything that looks like a long token, keeps the model's
// short reply and any error reason readable.
func redactBody(b []byte) string {
	s := string(b)
	var out []string
	for _, w := range strings.Fields(s) {
		if len(w) > 40 {
			out = append(out, "<redacted>")
		} else {
			out = append(out, w)
		}
	}
	return strings.Join(out, " ")
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
