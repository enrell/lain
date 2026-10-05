package torrent

import (
	"bufio"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
)

// Message ids (BEP 3, BEP 10).
const (
	msgChoke         byte = 0
	msgUnchoke       byte = 1
	msgInterested    byte = 2
	msgNotInterested byte = 3
	msgHave          byte = 4
	msgBitfield      byte = 5
	msgRequest       byte = 6
	msgPiece         byte = 7
	msgCancel        byte = 8
	msgExtended      byte = 20
)

const (
	protocolName = "BitTorrent protocol"
	blockSize    = 16 << 10
	// maxBlockRequest is the largest block we serve; larger requests are
	// a protocol violation and drop the peer.
	maxBlockRequest = 32 << 10
	// maxMessage bounds any message except a bitfield, which may be as
	// large as the torrent's piece count needs.
	maxMessage = 1 << 20
)

// ErrWire wraps protocol violations.
var ErrWire = errors.New("torrent: wire")

func wireErr(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrWire, fmt.Sprintf(format, args...))
}

// handshake is the 68-byte opening exchange.
type handshake struct {
	Reserved [8]byte
	InfoHash InfoHash
	PeerID   [20]byte
}

func (h handshake) supportsExtensions() bool { return h.Reserved[5]&0x10 != 0 }

func writeHandshake(w io.Writer, ih InfoHash, id [20]byte) error {
	b := make([]byte, 0, 68)
	b = append(b, byte(len(protocolName)))
	b = append(b, protocolName...)
	var reserved [8]byte
	reserved[5] |= 0x10 // BEP 10 extension protocol
	b = append(b, reserved[:]...)
	b = append(b, ih[:]...)
	b = append(b, id[:]...)
	_, err := w.Write(b)
	return err
}

func readHandshake(r io.Reader) (handshake, error) {
	var h handshake
	buf := make([]byte, 68)
	if _, err := io.ReadFull(r, buf); err != nil {
		return h, err
	}
	if buf[0] != byte(len(protocolName)) || string(buf[1:20]) != protocolName {
		return h, wireErr("not a BitTorrent handshake")
	}
	copy(h.Reserved[:], buf[20:28])
	copy(h.InfoHash[:], buf[28:48])
	copy(h.PeerID[:], buf[48:68])
	return h, nil
}

// message is one length-prefixed frame; keep-alives are not returned.
type message struct {
	ID      byte
	Payload []byte
}

// readMessage reads the next non-keepalive message. limit bounds the
// frame length.
func readMessage(r *bufio.Reader, limit int) (message, error) {
	for {
		var lenBuf [4]byte
		if _, err := io.ReadFull(r, lenBuf[:]); err != nil {
			return message{}, err
		}
		n := binary.BigEndian.Uint32(lenBuf[:])
		if n == 0 {
			continue // keep-alive
		}
		if int64(n) > int64(limit) {
			return message{}, wireErr("message of %d bytes exceeds %d", n, limit)
		}
		buf := make([]byte, n)
		if _, err := io.ReadFull(r, buf); err != nil {
			return message{}, err
		}
		return message{ID: buf[0], Payload: buf[1:]}, nil
	}
}

func frame(id byte, payload ...[]byte) []byte {
	n := 1
	for _, p := range payload {
		n += len(p)
	}
	b := make([]byte, 4, 4+n)
	binary.BigEndian.PutUint32(b, uint32(n))
	b = append(b, id)
	for _, p := range payload {
		b = append(b, p...)
	}
	return b
}

func u32s(vals ...uint32) []byte {
	b := make([]byte, 0, 4*len(vals))
	for _, v := range vals {
		b = binary.BigEndian.AppendUint32(b, v)
	}
	return b
}

// blockRef names one requested block.
type blockRef struct {
	Piece  int
	Begin  int
	Length int
}

func parseBlockRef(p []byte) (blockRef, error) {
	if len(p) != 12 {
		return blockRef{}, wireErr("request/cancel of %d bytes", len(p))
	}
	return blockRef{
		Piece:  int(binary.BigEndian.Uint32(p)),
		Begin:  int(binary.BigEndian.Uint32(p[4:])),
		Length: int(binary.BigEndian.Uint32(p[8:])),
	}, nil
}

func requestFrame(id byte, b blockRef) []byte {
	return frame(id, u32s(uint32(b.Piece), uint32(b.Begin), uint32(b.Length)))
}
