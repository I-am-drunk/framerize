// GET variant of the exact-fingerprint sender. Reads a JSON spec
// {addr, host, path, headers, out} from GET_SPEC_FILE, performs one HTTPS GET
// over the captured real-browser uTLS fingerprint, and writes the decoded
// response body to <out> (0600). Prints only status and byte counts.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"strconv"
	"strings"
	"time"

	tls "github.com/refraction-networking/utls"
)

func exactSpec() tls.ClientHelloSpec {
	spec, _ := tls.UTLSIdToSpec(tls.HelloChrome_133)
	var out []tls.TLSExtension
	for _, ext := range spec.Extensions {
		if _, ok := ext.(*tls.ApplicationSettingsExtensionNew); ok {
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
	specFile := os.Getenv("GET_SPEC_FILE")
	if specFile == "" {
		fmt.Println("get-skip: GET_SPEC_FILE unset")
		os.Exit(2)
	}
	data, err := os.ReadFile(specFile)
	if err != nil {
		fmt.Println("get-read-error:", err)
		os.Exit(1)
	}
	var req struct {
		Addr    string            `json:"addr"`
		Host    string            `json:"host"`
		Path    string            `json:"path"`
		Headers map[string]string `json:"headers"`
		Out     string            `json:"out"`
	}
	if err := json.Unmarshal(data, &req); err != nil {
		fmt.Println("get-parse-error:", err)
		os.Exit(1)
	}
	status, chunked, body, err := sendGet(req.Addr, req.Host, req.Path, req.Headers)
	if err != nil {
		fmt.Println("get-send-error:", err)
		os.Exit(1)
	}
	if chunked {
		body = dechunk(body)
	}
	if req.Out != "" {
		if werr := os.WriteFile(req.Out, body, 0600); werr != nil {
			fmt.Println("get-write-error:", werr)
			os.Exit(1)
		}
	}
	fmt.Printf("get-status=%d chunked=%v body-bytes=%d\n", status, chunked, len(body))
}

func sendGet(addr, host, path string, headers map[string]string) (int, bool, []byte, error) {
	spec := exactSpec()
	dial, err := net.DialTimeout("tcp", addr, 8*time.Second)
	if err != nil {
		return 0, false, nil, err
	}
	cfg := &tls.Config{ServerName: host, InsecureSkipVerify: true, NextProtos: []string{"http/1.1"}}
	conn := tls.UClient(dial, cfg, tls.HelloCustom)
	if err := conn.ApplyPreset(&spec); err != nil {
		return 0, false, nil, err
	}
	if err := conn.Handshake(); err != nil {
		return 0, false, nil, err
	}
	defer conn.Close()
	_ = conn.SetReadDeadline(time.Now().Add(10 * time.Second))

	req := "GET " + path + " HTTP/1.1\r\nHost: " + host + "\r\n"
	for k, v := range headers {
		req += k + ": " + v + "\r\n"
	}
	req += "Connection: close\r\n\r\n"
	if _, err := conn.Write([]byte(req)); err != nil {
		return 0, false, nil, err
	}
	var buf []byte
	tmp := make([]byte, 8192)
	for {
		n, err := conn.Read(tmp)
		if n > 0 {
			buf = append(buf, tmp[:n]...)
		}
		if err != nil {
			break
		}
	}
	return splitResponse(buf)
}

// splitResponse splits raw bytes into status, chunked-flag, and body.
func splitResponse(buf []byte) (int, bool, []byte, error) {
	idx := bytes.Index(buf, []byte("\r\n\r\n"))
	if idx < 0 {
		return 0, false, nil, fmt.Errorf("no header terminator")
	}
	head := string(buf[:idx])
	body := buf[idx+4:]
	status := 0
	fmt.Sscanf(head, "HTTP/1.1 %d", &status)
	chunked := false
	for _, line := range strings.Split(head, "\r\n") {
		if strings.HasPrefix(strings.ToLower(line), "transfer-encoding:") &&
			strings.Contains(strings.ToLower(line), "chunked") {
			chunked = true
		}
	}
	return status, chunked, body, nil
}

// dechunk decodes an HTTP/1.1 chunked body.
func dechunk(b []byte) []byte {
	var out []byte
	for len(b) > 0 {
		nl := bytes.Index(b, []byte("\r\n"))
		if nl < 0 {
			break
		}
		sizeStr := strings.TrimSpace(string(b[:nl]))
		if i := strings.Index(sizeStr, ";"); i >= 0 {
			sizeStr = sizeStr[:i]
		}
		size, err := strconv.ParseInt(sizeStr, 16, 64)
		if err != nil || size < 0 {
			break
		}
		b = b[nl+2:]
		if size == 0 {
			break
		}
		if int64(len(b)) < size {
			out = append(out, b...)
			break
		}
		out = append(out, b[:size]...)
		b = b[size:]
		if len(b) >= 2 {
			b = b[2:] // trailing CRLF after each chunk
		}
	}
	return out
}
