// Build a ClientHelloSpec that replays the captured real-browser shape exactly,
// including the new ApplicationSettings codepoint 51764 (Chrome's ALPS draft),
// which the stock HelloChrome_133 parrot emits on the older 17613 instead.
package main

import (
	"fmt"
	"net"
	"os"
	"time"

	tls "github.com/refraction-networking/utls"
)

func exactSpec() tls.ClientHelloSpec {
	// Start from the real browser parrot, then replace the ALPS extension with
	// the new codepoint so the extension set matches the captured browser.
	spec, _ := tls.UTLSIdToSpec(tls.HelloChrome_133)
	// The captured real browser emits ALPS on BOTH codepoints 51764 (new) and
	// 17613 (old), with the same h2 payload. Replace the parrot's 17613 entry
	// with both, preserving its position.
	var out []tls.TLSExtension
	for _, ext := range spec.Extensions {
		if _, ok := ext.(*tls.ApplicationSettingsExtensionNew); ok {
			// ALPS h2 payload: 2-byte list len, then one 1-byte-len "h2".
			payload := []byte{0x00, 0x03, 0x02, 'h', '2'}
			out = append(out, &tls.GenericExtension{Id: 51764, Data: payload})
			out = append(out, &tls.GenericExtension{Id: 17613, Data: payload})
			continue
		}
		out = append(out, ext)
	}
	spec.Extensions = out
	return spec
}

func main() {
	// Send mode: skip the capture handshake and go straight to the POST.
	if os.Getenv("SEND_ADDR") != "" {
		runSend()
		return
	}
	addr := os.Getenv("CAPTURE_ADDR")
	if addr == "" {
		addr = "127.0.0.1:8443"
	}
	spec := exactSpec()
	dial, err := net.DialTimeout("tcp", addr, 3*time.Second)
	if err != nil {
		fmt.Println("dial-error:", err)
		os.Exit(1)
	}
	cfg := &tls.Config{ServerName: "localhost", InsecureSkipVerify: true, NextProtos: []string{"h2", "http/1.1"}}
	conn := tls.UClient(dial, cfg, tls.HelloCustom)
	if err := conn.ApplyPreset(&spec); err != nil {
		fmt.Println("apply-error:", err)
		os.Exit(1)
	}
	_ = conn.Handshake()
	fmt.Println("handshake-attempted with exact spec")
	conn.Close()
}

// runSend is the offline path-test entry: POST a body to a local echo server
// over the uTLS fingerprint and report the status. No external request.
func runSend() {
	addr := os.Getenv("SEND_ADDR") // host:port of the echo server
	host := os.Getenv("SEND_HOST") // TLS servername
	if addr == "" || host == "" {
		fmt.Println("send-skip: SEND_ADDR/SEND_HOST unset")
		return
	}
	body := []byte(os.Getenv("SEND_BODY"))
	status, head, err := sendPost(addr, host, "/", map[string]string{
		"Content-Type": "application/json",
	}, body)
	if err != nil {
		fmt.Println("send-error:", err)
		return
	}
	fmt.Printf("send-status=%d head=%q\n", status, head[:min(120, len(head))])
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// sendPost performs one HTTPS POST through the uTLS fingerprint to a target
// address. It is exercised against a local echo server in the offline path
// test; the production target is api.framer.com. Returns status and body head.
func sendPost(addr, host, path string, headers map[string]string, body []byte) (int, string, error) {
	spec := exactSpec()
	dial, err := net.DialTimeout("tcp", addr, 5*time.Second)
	if err != nil {
		return 0, "", err
	}
	cfg := &tls.Config{ServerName: host, InsecureSkipVerify: true, NextProtos: []string{"http/1.1"}}
	conn := tls.UClient(dial, cfg, tls.HelloCustom)
	if err := conn.ApplyPreset(&spec); err != nil {
		return 0, "", err
	}
	if err := conn.Handshake(); err != nil {
		return 0, "", err
	}
	defer conn.Close()

	req := "POST " + path + " HTTP/1.1\r\nHost: " + host + "\r\n"
	for k, v := range headers {
		req += k + ": " + v + "\r\n"
	}
	req += fmt.Sprintf("Content-Length: %d\r\nConnection: close\r\n\r\n", len(body))
	if _, err := conn.Write([]byte(req)); err != nil {
		return 0, "", err
	}
	if _, err := conn.Write(body); err != nil {
		return 0, "", err
	}

	// Read the status line and a small head of the response.
	buf := make([]byte, 4096)
	n, err := conn.Read(buf)
	if err != nil && n == 0 {
		return 0, "", err
	}
	head := string(buf[:n])
	status := 0
	fmt.Sscanf(head, "HTTP/1.1 %d", &status)
	return status, head, nil
}
