package localplay

import (
	"fmt"
	"io"
	"net"
	"strconv"
	"strings"
	"time"
)

// VLC's RC interface over a private unix socket reports the active
// playlist title, position and duration. Commands never carry tokens or
// URLs here: the playlist references local file paths only.

func vlcRC(socket, command string) (string, error) {
	conn, err := net.DialTimeout("unix", socket, 300*time.Millisecond)
	if err != nil {
		return "", err
	}
	defer conn.Close()
	_ = conn.SetDeadline(time.Now().Add(700 * time.Millisecond))
	if _, err := io.WriteString(conn, command); err != nil {
		return "", err
	}
	var output strings.Builder
	buffer := make([]byte, 4096)
	for {
		n, err := conn.Read(buffer)
		if n > 0 {
			output.Write(buffer[:n])
		}
		if err != nil {
			if timeout, ok := err.(net.Error); ok && timeout.Timeout() && output.Len() > 0 {
				return output.String(), nil
			}
			if err == io.EOF && output.Len() > 0 {
				return output.String(), nil
			}
			return "", err
		}
	}
}

// vlcQueueIndex maps VLC's current playlist title to the entry index.
// The m3u marks each entry `lain-episode-%06d`; an unrecognized title is
// not an error, just "nothing known playing yet".
func vlcQueueIndex(socket string, count int) (int, error) {
	title, err := vlcRC(socket, "get_title\n")
	if err != nil {
		return -1, err
	}
	for i := 0; i < count; i++ {
		if strings.Contains(title, fmt.Sprintf("lain-episode-%06d", i)) {
			return i, nil
		}
	}
	return -1, nil
}

func vlcPosition(socket string) (float64, float64, error) {
	response, err := vlcRC(socket, "get_time\nget_length\n")
	if err != nil {
		return 0, 0, err
	}
	var values []float64
	for _, line := range strings.Split(response, "\n") {
		line = strings.TrimSpace(strings.TrimLeft(line, "> "))
		if n, err := strconv.ParseFloat(line, 64); err == nil && n >= 0 {
			values = append(values, n)
			if len(values) == 2 {
				return values[0], values[1], nil
			}
		}
	}
	return 0, 0, fmt.Errorf("VLC did not report time and duration")
}

func vlcSeek(socket string, seconds float64) error {
	_, err := vlcRC(socket, "seek "+strconv.FormatFloat(seconds, 'f', 0, 64)+"\n")
	return err
}
