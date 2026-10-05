package torrent

import (
	"bufio"
	"context"
	"crypto/sha1"
	"encoding/binary"
	"net"
	"net/netip"
	"sync"
	"time"

	"github.com/enrell/lain/internal/torrent/bencode"
)

const (
	maxInflight     = 32
	maxUploadQueue  = 256
	maxMetadataSize = 8 << 20
	metaPieceSize   = 16 << 10
	localUtMetadata = 1 // the id peers use to send us ut_metadata
	endgameBlocks   = 32
)

type peer struct {
	t      *Torrent
	ctx    context.Context
	conn   net.Conn
	r      *bufio.Reader
	addr   netip.AddrPort
	id     [20]byte
	ext    bool
	out    chan []byte
	upload chan blockRef
	quit   chan struct{}
	once   sync.Once

	// Guarded by t.mu.
	amChoking, amInterested     bool
	peerChoking, peerInterested bool
	bf                          Bitfield
	inflight                    map[blockKey]time.Time
	canceled                    map[blockRef]bool
	utMeta                      byte
	unchokedAt, interestedAt    time.Time
	bad                         int
}

// attach registers a handshaken connection and starts its loops.
func (t *Torrent) attach(conn net.Conn, r *bufio.Reader, hs handshake, addr netip.AddrPort) {
	p := &peer{
		t: t, conn: conn, r: r, addr: addr, id: hs.PeerID, ext: hs.supportsExtensions(),
		out: make(chan []byte, 512), upload: make(chan blockRef, maxUploadQueue), quit: make(chan struct{}),
		amChoking: true, peerChoking: true,
		inflight: map[blockKey]time.Time{}, canceled: map[blockRef]bool{},
	}
	t.mu.Lock()
	// Peers that arrive during the hash check are refused: the bitfield
	// is only sent on attach and would announce pieces still unknown.
	if !t.running || t.state == StateChecking || len(t.peers) >= t.c.cfg.MaxPeersPerTorrent {
		t.mu.Unlock()
		conn.Close()
		return
	}
	for q := range t.peers {
		if q.id == p.id {
			t.mu.Unlock()
			conn.Close()
			return
		}
	}
	t.peers[p] = true
	p.ctx = t.ctx
	// Added under the lock while running, so stop's Wait cannot race it.
	t.wg.Add(2)
	if _, ok := t.known[addr]; ok {
		delete(t.backoff, addr) // reachable again: retry quickly if it drops
	} else if addr.IsValid() {
		// Inbound peers' source ports are ephemeral; never redial them.
		t.known[addr] = time.Now().Add(365 * 24 * time.Hour)
	}
	if p.ext {
		m := map[string]any{"m": map[string]any{"ut_metadata": localUtMetadata}, "v": "Lain", "reqq": maxUploadQueue}
		if t.info != nil {
			m["metadata_size"] = len(t.info.Raw)
		}
		if raw, err := bencode.Encode(m); err == nil {
			p.send(frame(msgExtended, []byte{0}, raw))
		}
	}
	if t.info != nil && t.have.Count(t.info.NumPieces()) > 0 {
		p.send(frame(msgBitfield, t.have))
	}
	t.mu.Unlock()
	go p.writeLoop()
	go p.readLoop()
}

// send queues a control frame; a peer that cannot keep up is dropped.
// Callers hold t.mu or own the peer exclusively.
func (p *peer) send(b []byte) {
	select {
	case p.out <- b:
	default:
		go p.close()
	}
}

func (p *peer) close() {
	p.once.Do(func() {
		close(p.quit)
		p.conn.Close()
	})
}

func (p *peer) setChoking(choke bool) {
	if p.amChoking == choke {
		return
	}
	p.amChoking = choke
	if choke {
		p.send(frame(msgChoke))
	} else {
		p.unchokedAt = time.Now()
		p.send(frame(msgUnchoke))
	}
}

func (p *peer) writeLoop() {
	defer p.t.wg.Done()
	keep := time.NewTicker(90 * time.Second)
	defer keep.Stop()
	write := func(b []byte) bool {
		_ = p.conn.SetWriteDeadline(time.Now().Add(60 * time.Second))
		_, err := p.conn.Write(b)
		return err == nil
	}
	for {
		select {
		case <-p.quit:
			return
		case b := <-p.out:
			if !write(b) {
				p.close()
				return
			}
		case req := <-p.upload:
			data, ok := p.t.readBlockForUpload(p, req)
			if !ok {
				continue
			}
			if err := p.t.c.up.Wait(p.ctx, len(data)); err != nil {
				p.close()
				return
			}
			if !write(frame(msgPiece, u32s(uint32(req.Piece), uint32(req.Begin)), data)) {
				p.close()
				return
			}
			p.t.mu.Lock()
			p.t.uploaded += int64(len(data))
			p.t.mu.Unlock()
		case <-keep.C:
			if !write([]byte{0, 0, 0, 0}) {
				p.close()
				return
			}
		}
	}
}

func (p *peer) readLoop() {
	defer p.t.wg.Done()
	defer p.t.detach(p)
	for {
		limit := maxMessage
		p.t.mu.Lock()
		if p.t.info != nil {
			limit = max(limit, len(p.t.have)+1)
		}
		p.t.mu.Unlock()
		_ = p.conn.SetReadDeadline(time.Now().Add(3 * time.Minute))
		m, err := readMessage(p.r, limit)
		if err != nil {
			p.close()
			return
		}
		if err := p.t.handle(p, m); err != nil {
			p.t.c.cfg.Logger.Info("dropping peer", "torrent", p.t.ih.Hex(), "peer", p.addr.String(), "err", err.Error())
			p.close()
			return
		}
	}
}

// detach forgets a closed peer and releases its requests.
func (t *Torrent) detach(p *peer) {
	p.close()
	t.mu.Lock()
	defer t.mu.Unlock()
	delete(t.peers, p)
	for k := range p.inflight {
		t.dropRequestLocked(p, k)
	}
	if t.info != nil && p.bf != nil {
		for i := 0; i < t.info.NumPieces(); i++ {
			if p.bf.Has(i) {
				t.avail[i]--
			}
		}
	}
	if !p.amChoking {
		t.rechokeLocked(false)
	}
	t.poke()
}

// handle processes one message from p.
func (t *Torrent) handle(p *peer, m message) error {
	switch m.ID {
	case msgChoke:
		t.mu.Lock()
		p.peerChoking = true
		for k := range p.inflight {
			t.dropRequestLocked(p, k)
		}
		t.mu.Unlock()
	case msgUnchoke:
		t.mu.Lock()
		p.peerChoking = false
		t.fillLocked(p)
		t.mu.Unlock()
	case msgInterested:
		t.mu.Lock()
		if !p.peerInterested {
			p.peerInterested = true
			p.interestedAt = time.Now()
			t.rechokeLocked(false)
		}
		t.mu.Unlock()
	case msgNotInterested:
		t.mu.Lock()
		p.peerInterested = false
		t.rechokeLocked(false)
		t.mu.Unlock()
	case msgHave:
		if len(m.Payload) != 4 {
			return wireErr("have of %d bytes", len(m.Payload))
		}
		i := int(binary.BigEndian.Uint32(m.Payload))
		t.mu.Lock()
		defer t.mu.Unlock()
		return t.peerHasLocked(p, i)
	case msgBitfield:
		t.mu.Lock()
		defer t.mu.Unlock()
		if p.bf != nil && p.bf.Count(len(p.bf)*8) > 0 {
			return wireErr("late bitfield")
		}
		bf := Bitfield(append([]byte(nil), m.Payload...))
		if t.info != nil {
			n := t.info.NumPieces()
			if !bf.Valid(n) {
				return wireErr("bitfield does not fit %d pieces", n)
			}
			for i := 0; i < n; i++ {
				if bf.Has(i) {
					t.avail[i]++
				}
			}
		} else if len(bf) > 1<<21 {
			return wireErr("bitfield too large")
		}
		p.bf = bf
		t.updateInterestLocked(p)
		t.fillLocked(p)
	case msgRequest:
		ref, err := parseBlockRef(m.Payload)
		if err != nil {
			return err
		}
		t.mu.Lock()
		defer t.mu.Unlock()
		if t.info == nil || ref.Piece < 0 || ref.Piece >= t.info.NumPieces() || ref.Length <= 0 || ref.Length > maxBlockRequest ||
			ref.Begin < 0 || int64(ref.Begin)+int64(ref.Length) > t.info.PieceSize(ref.Piece) {
			return wireErr("bad request %+v", ref)
		}
		if p.amChoking || !t.have.Has(ref.Piece) {
			return nil // ignored, not fatal: races with choke are normal
		}
		delete(p.canceled, ref)
		select {
		case p.upload <- ref:
		default: // queue full: the peer re-requests later
		}
	case msgCancel:
		ref, err := parseBlockRef(m.Payload)
		if err != nil {
			return err
		}
		t.mu.Lock()
		if len(p.canceled) < 4*maxUploadQueue {
			p.canceled[ref] = true
		}
		t.mu.Unlock()
	case msgPiece:
		return t.receiveBlock(p, m.Payload)
	case msgExtended:
		return t.handleExtended(p, m.Payload)
	default:
		// Unknown and unsupported messages (port, fast extension) are
		// ignored, as BEP 3 allows.
	}
	return nil
}

func (t *Torrent) peerHasLocked(p *peer, i int) error {
	if t.info != nil {
		if i < 0 || i >= t.info.NumPieces() {
			return wireErr("have %d out of range", i)
		}
		if p.bf == nil {
			p.bf = NewBitfield(t.info.NumPieces())
		}
		if !p.bf.Has(i) {
			p.bf.Set(i)
			t.avail[i]++
		}
		t.updateInterestLocked(p)
		t.fillLocked(p)
		return nil
	}
	if i < 0 || i >= 1<<24 {
		return wireErr("have %d out of range", i)
	}
	for len(p.bf)*8 <= i {
		p.bf = append(p.bf, 0)
	}
	p.bf.Set(i)
	return nil
}

func (t *Torrent) updateInterestLocked(p *peer) {
	want := false
	if t.info != nil && !t.completed && p.bf != nil {
		for i := 0; i < t.info.NumPieces(); i++ {
			if p.bf.Has(i) && !t.have.Has(i) {
				want = true
				break
			}
		}
	}
	if want != p.amInterested {
		p.amInterested = want
		if want {
			p.send(frame(msgInterested))
		} else {
			p.send(frame(msgNotInterested))
		}
	}
}

// fillLocked keeps p's request pipeline full: partial pieces first,
// then the rarest piece p has; endgame duplicates the last blocks.
func (t *Torrent) fillLocked(p *peer) {
	if t.info == nil || t.completed || p.peerChoking || !p.amInterested || p.bf == nil {
		return
	}
	for len(p.inflight) < maxInflight {
		ref, ok := t.pickLocked(p)
		if !ok {
			return
		}
		k := blockKey{ref.Piece, ref.Begin}
		p.inflight[k] = time.Now()
		ps := &t.pieces[ref.Piece]
		b := ref.Begin / blockSize
		ps.reqs[b] = append(ps.reqs[b], p)
		t.active[ref.Piece] = true
		p.send(requestFrame(msgRequest, ref))
	}
}

func (t *Torrent) blockRefLocked(piece, b int) blockRef {
	begin := b * blockSize
	length := int(min(int64(blockSize), t.info.PieceSize(piece)-int64(begin)))
	return blockRef{Piece: piece, Begin: begin, Length: length}
}

func (t *Torrent) pickLocked(p *peer) (blockRef, bool) {
	free := func(i int) (blockRef, bool) {
		ps := &t.pieces[i]
		if ps.verifying || t.have.Has(i) || !p.bf.Has(i) {
			return blockRef{}, false
		}
		for b, got := range ps.got {
			if !got && len(ps.reqs[b]) == 0 {
				return t.blockRefLocked(i, b), true
			}
		}
		return blockRef{}, false
	}
	for i := range t.active {
		if ref, ok := free(i); ok {
			return ref, true
		}
	}
	best, bestAvail := -1, 0
	for i := 0; i < t.info.NumPieces(); i++ {
		if t.active[i] || t.have.Has(i) || !p.bf.Has(i) {
			continue
		}
		if best < 0 || t.avail[i] < bestAvail {
			best, bestAvail = i, t.avail[i]
		}
	}
	if best >= 0 {
		return free(best)
	}
	// Endgame: everything is requested; duplicate the remaining blocks
	// this peer is not already fetching.
	remaining := 0
	for i := range t.active {
		ps := &t.pieces[i]
		remaining += len(ps.got) - ps.gotN
	}
	if remaining > endgameBlocks {
		return blockRef{}, false
	}
	for i := range t.active {
		ps := &t.pieces[i]
		if ps.verifying || !p.bf.Has(i) {
			continue
		}
		for b, got := range ps.got {
			if got {
				continue
			}
			if _, mine := p.inflight[blockKey{i, b * blockSize}]; !mine {
				return t.blockRefLocked(i, b), true
			}
		}
	}
	return blockRef{}, false
}

// receiveBlock stores a block, and verifies the piece once complete.
func (t *Torrent) receiveBlock(p *peer, payload []byte) error {
	if len(payload) < 8 {
		return wireErr("short piece message")
	}
	piece := int(binary.BigEndian.Uint32(payload))
	begin := int(binary.BigEndian.Uint32(payload[4:]))
	data := payload[8:]

	t.mu.Lock()
	if t.info == nil || piece < 0 || piece >= t.info.NumPieces() || begin%blockSize != 0 {
		t.mu.Unlock()
		return wireErr("unexpected block")
	}
	want := t.blockRefLocked(piece, begin/blockSize)
	if begin >= int(t.info.PieceSize(piece)) || len(data) != want.Length {
		t.mu.Unlock()
		return wireErr("block of wrong size")
	}
	ps := &t.pieces[piece]
	b := begin / blockSize
	k := blockKey{piece, begin}
	_, requested := p.inflight[k]
	if !requested || ps.got[b] || ps.verifying || t.have.Has(piece) {
		// Unrequested, a late duplicate, or after a cancel: discard.
		if requested {
			t.dropRequestLocked(p, k)
		}
		t.mu.Unlock()
		return nil
	}
	st := t.st
	t.mu.Unlock()

	if err := t.c.down.Wait(p.ctx, len(data)); err != nil {
		return err
	}
	if err := st.WriteAt(data, int64(piece)*t.info.PieceLength+int64(begin)); err != nil {
		t.fail("write failed: " + err.Error())
		return err
	}

	t.mu.Lock()
	t.downloaded += int64(len(data))
	if ps.got[b] || t.have.Has(piece) {
		t.dropRequestLocked(p, k)
		t.mu.Unlock()
		return nil
	}
	ps.got[b] = true
	ps.gotN++
	ps.contribute[p] = true
	// Cancel duplicates still out with other peers (endgame).
	for _, q := range ps.reqs[b] {
		if q != p {
			q.send(requestFrame(msgCancel, want))
			delete(q.inflight, k)
		}
	}
	delete(ps.reqs, b)
	delete(p.inflight, k)
	complete := ps.gotN == len(ps.got)
	if complete {
		ps.verifying = true
	}
	t.fillLocked(p)
	t.mu.Unlock()

	if complete {
		t.verifyPiece(piece)
	}
	return nil
}

func (t *Torrent) verifyPiece(piece int) {
	ok, err := t.st.Verify(piece)
	if err != nil {
		t.fail("read failed: " + err.Error())
		return
	}
	t.mu.Lock()
	ps := &t.pieces[piece]
	ps.verifying = false
	delete(t.active, piece)
	if !ok {
		ps.failures++
		for q := range ps.contribute {
			q.bad++
			if q.bad >= 3 {
				go q.close() // repeatedly sent corrupt data
			}
		}
		for i := range ps.got {
			ps.got[i] = false
		}
		ps.gotN = 0
		ps.contribute = map[*peer]bool{}
		for q := range t.peers {
			t.fillLocked(q)
		}
		t.mu.Unlock()
		return
	}
	t.have.Set(piece)
	ps.contribute = map[*peer]bool{}
	have := frame(msgHave, u32s(uint32(piece)))
	for q := range t.peers {
		q.send(have)
		t.updateInterestLocked(q)
	}
	done := t.have.Count(t.info.NumPieces()) == t.info.NumPieces()
	if done && !t.completed {
		t.completed = true
		t.state = StateSeeding
		for q := range t.peers {
			t.updateInterestLocked(q)
		}
	} else {
		for q := range t.peers {
			t.fillLocked(q)
		}
		done = false
	}
	t.mu.Unlock()
	if done {
		t.c.cfg.Logger.Info("torrent complete", "torrent", t.ih.Hex())
		t.finalize(true)
		t.poke()
	}
}

// readBlockForUpload reads a requested block if it is still wanted.
func (t *Torrent) readBlockForUpload(p *peer, ref blockRef) ([]byte, bool) {
	t.mu.Lock()
	if p.canceled[ref] || p.amChoking || t.info == nil || !t.have.Has(ref.Piece) {
		delete(p.canceled, ref)
		t.mu.Unlock()
		return nil, false
	}
	st, plen := t.st, t.info.PieceLength
	t.mu.Unlock()
	buf := make([]byte, ref.Length)
	if err := st.ReadAt(buf, int64(ref.Piece)*plen+int64(ref.Begin)); err != nil {
		return nil, false
	}
	return buf, true
}

// handleExtended speaks BEP 10 and BEP 9 (ut_metadata).
func (t *Torrent) handleExtended(p *peer, payload []byte) error {
	if len(payload) < 1 {
		return wireErr("empty extended message")
	}
	if payload[0] == 0 {
		v, err := bencode.Decode(payload[1:])
		if err != nil {
			return wireErr("extended handshake: %v", err)
		}
		d, _ := v.(map[string]any)
		t.mu.Lock()
		defer t.mu.Unlock()
		if mm, ok := bencode.Dict(d, "m"); ok {
			if id, ok := bencode.Int(mm, "ut_metadata"); ok && id > 0 && id < 256 {
				p.utMeta = byte(id)
			}
		}
		if size, ok := bencode.Int(d, "metadata_size"); ok && t.info == nil && size > 0 && size <= maxMetadataSize {
			if t.metaSize == 0 {
				t.metaSize = int(size)
				t.metaPieces = make([][]byte, (int(size)+metaPieceSize-1)/metaPieceSize)
			}
		}
		t.requestMetaLocked()
		return nil
	}
	if payload[0] != localUtMetadata {
		return nil // an extension we did not advertise
	}
	v, rest, err := bencode.DecodePrefix(payload[1:])
	if err != nil {
		return wireErr("ut_metadata: %v", err)
	}
	d, _ := v.(map[string]any)
	typ, _ := bencode.Int(d, "msg_type")
	piece, _ := bencode.Int(d, "piece")
	t.mu.Lock()
	defer t.mu.Unlock()
	switch typ {
	case 0: // request
		if p.utMeta == 0 {
			return nil
		}
		if t.info == nil || piece < 0 || int(piece)*metaPieceSize >= len(t.info.Raw) {
			raw, _ := bencode.Encode(map[string]any{"msg_type": 2, "piece": piece})
			p.send(frame(msgExtended, []byte{p.utMeta}, raw))
			return nil
		}
		start := int(piece) * metaPieceSize
		end := min(start+metaPieceSize, len(t.info.Raw))
		raw, _ := bencode.Encode(map[string]any{"msg_type": 1, "piece": piece, "total_size": len(t.info.Raw)})
		p.send(frame(msgExtended, []byte{p.utMeta}, raw, t.info.Raw[start:end]))
	case 1: // data
		if t.info != nil || t.metaSize == 0 || piece < 0 || int(piece) >= len(t.metaPieces) {
			return nil
		}
		want := min(metaPieceSize, t.metaSize-int(piece)*metaPieceSize)
		if len(rest) != want {
			return wireErr("metadata piece of %d bytes, want %d", len(rest), want)
		}
		t.metaPieces[piece] = append([]byte(nil), rest...)
		delete(t.metaAsked, int(piece))
		t.tryMetadataLocked()
	case 2: // reject
		delete(t.metaAsked, int(piece))
	}
	return nil
}

func (t *Torrent) requestMetaLocked() {
	if t.info != nil || t.metaSize == 0 {
		return
	}
	var sources []*peer
	for q := range t.peers {
		if q.utMeta != 0 {
			sources = append(sources, q)
		}
	}
	if len(sources) == 0 {
		return
	}
	n := 0
	for i, data := range t.metaPieces {
		if data != nil {
			continue
		}
		if _, asked := t.metaAsked[i]; asked {
			continue
		}
		q := sources[n%len(sources)]
		n++
		raw, _ := bencode.Encode(map[string]any{"msg_type": 0, "piece": i})
		q.send(frame(msgExtended, []byte{q.utMeta}, raw))
		t.metaAsked[i] = time.Now()
	}
}

func (t *Torrent) expireMetaLocked() {
	for i, at := range t.metaAsked {
		if time.Since(at) > 20*time.Second {
			delete(t.metaAsked, i)
		}
	}
	t.requestMetaLocked()
}

func (t *Torrent) tryMetadataLocked() {
	var raw []byte
	for _, d := range t.metaPieces {
		if d == nil {
			return
		}
		raw = append(raw, d...)
	}
	if sha1.Sum(raw) != t.ih {
		// A peer lied; start over.
		t.metaPieces = make([][]byte, len(t.metaPieces))
		t.metaAsked = map[int]time.Time{}
		return
	}
	info, err := ParseInfo(raw)
	if err != nil {
		t.state = StateError
		t.err = "metadata: " + err.Error()
		return
	}
	t.setInfoLocked(info)
	t.state = StateDownloading
	t.metaPieces, t.metaAsked = nil, map[int]time.Time{}
	for q := range t.peers {
		t.updateInterestLocked(q)
		t.fillLocked(q)
	}
	if t.onMetadata != nil {
		go t.onMetadata(t)
	}
}
