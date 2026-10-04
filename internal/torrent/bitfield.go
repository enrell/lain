package torrent

// Bitfield is a piece set in wire order: piece 0 is the high bit of
// byte 0 (BEP 3).
type Bitfield []byte

// NewBitfield makes an empty set for n pieces.
func NewBitfield(n int) Bitfield { return make(Bitfield, (n+7)/8) }

// Has reports piece i.
func (b Bitfield) Has(i int) bool {
	if i < 0 || i/8 >= len(b) {
		return false
	}
	return b[i/8]&(0x80>>(i%8)) != 0
}

// Set adds piece i.
func (b Bitfield) Set(i int) {
	if i >= 0 && i/8 < len(b) {
		b[i/8] |= 0x80 >> (i % 8)
	}
}

// Clear removes piece i.
func (b Bitfield) Clear(i int) {
	if i >= 0 && i/8 < len(b) {
		b[i/8] &^= 0x80 >> (i % 8)
	}
}

// Count is the number of pieces set among the first n.
func (b Bitfield) Count(n int) int {
	c := 0
	for i := 0; i < n; i++ {
		if b.Has(i) {
			c++
		}
	}
	return c
}

// Valid reports whether a peer's bitfield fits n pieces: right length
// and no spare bits set (BEP 3 says drop such peers).
func (b Bitfield) Valid(n int) bool {
	if len(b) != (n+7)/8 {
		return false
	}
	for i := n; i < len(b)*8; i++ {
		if b.Has(i) {
			return false
		}
	}
	return true
}

// Clone copies the set.
func (b Bitfield) Clone() Bitfield { return append(Bitfield(nil), b...) }
