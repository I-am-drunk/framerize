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
