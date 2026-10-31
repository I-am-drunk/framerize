// Emit a uTLS Chrome ClientHello to a local capture server and report what was
// sent, so we can diff the JA3 against the real browser's fingerprint offline.
// No external request is made; the target is a loopback listener we control.
package main

import (
	"fmt"
	"net"
	"os"
	"time"

	tls "github.com/refraction-networking/utls"
)

func main() {
	addr := os.Getenv("CAPTURE_ADDR")
	if addr == "" {
		addr = "127.0.0.1:8443"
	}
	spec := tls.HelloChrome_133
	dial, err := net.DialTimeout("tcp", addr, 3*time.Second)
	if err != nil {
		fmt.Println("dial-error:", err)
		os.Exit(1)
	}
	cfg := &tls.Config{ServerName: "api.framer.com", InsecureSkipVerify: true}
	conn := tls.UClient(dial, cfg, spec)
	// Trigger the handshake; the capture server will drop us, so ignore errors.
	_ = conn.Handshake()
	fmt.Println("handshake-attempted with", spec.Client, spec.Version)
	conn.Close()
}
