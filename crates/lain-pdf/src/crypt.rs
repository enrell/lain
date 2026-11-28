//! The standard security handler (§7.6.3, ISO 32000-2 §7.6.4): opens a
//! document whose user password is empty, the usual case for books
//! that only restrict printing or copying. RC4, AES (128 and 256),
//! MD5 and the revision 6 hash are here; SHA-2 comes from `sha2`.

use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::error::{PdfError, Result};
use crate::object::{Dict, ObjRef, Object};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00,
    0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    None,
    Rc4,
    Aes128,
    Aes256,
}

/// A document's decryption state once the password checked out.
#[derive(Debug, Clone)]
pub struct Decryptor {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    pub encrypt_metadata: bool,
    /// The `/Encrypt` dictionary's own object, never decrypted.
    pub encrypt_ref: Option<ObjRef>,
}

impl Decryptor {
    /// Opens `/Encrypt` with the empty password, as the user or as the
    /// owner. `id0` is the first element of the trailer's `/ID`.
    pub fn open(enc: &Dict, id0: &[u8], encrypt_ref: Option<ObjRef>) -> Result<Decryptor> {
        if enc.name("Filter").is_some_and(|f| f != "Standard") {
            return Err(PdfError::unsupported(format!("security handler {}", enc.name("Filter").unwrap_or(""))));
        }
        let int = |k: &str| enc.get(k).and_then(Object::as_i64);
        let v = int("V").unwrap_or(0);
        let r = int("R").unwrap_or(2);
        let bytes = |k: &str| enc.get(k).and_then(Object::as_bytes).map(<[u8]>::to_vec).unwrap_or_default();
        let (o, u) = (bytes("O"), bytes("U"));
        let p = int("P").unwrap_or(-1) as i32;
        let encrypt_metadata = enc.get("EncryptMetadata").and_then(Object::as_bool).unwrap_or(true);
        let (strings, streams) = methods(enc, v);
        let key = if r >= 5 {
            let key = key_r6(r, &o, &u, &bytes("OE"), &bytes("UE"))
                .ok_or_else(|| PdfError::encrypted("the document needs a password"))?;
            key.to_vec()
        } else {
            let len = match v {
                1 => 5,
                _ => (int("Length").unwrap_or(40).clamp(40, 128) / 8) as usize,
            };
            let len = if r == 2 { 5 } else { len };
            let user = key_r4(b"", r, len, &o, p, id0, encrypt_metadata);
            if check_user(&user, r, &u, id0) {
                user
            } else {
                // An empty owner password unlocks the user password.
                let pw = owner_to_user(b"", r, len, &o);
                let k = key_r4(&pw, r, len, &o, p, id0, encrypt_metadata);
                if !check_user(&k, r, &u, id0) {
                    return Err(PdfError::encrypted("the document needs a password"));
                }
                k
            }
        };
        Ok(Decryptor { key, strings, streams, encrypt_metadata, encrypt_ref })
    }

    fn object_key(&self, id: ObjRef, aes: bool) -> Vec<u8> {
        if self.key.len() == 32 {
            return self.key.clone();
        }
        let mut m = Md5::new();
        m.update(&self.key);
        m.update(&id.num.to_le_bytes()[..3]);
        m.update(&id.generation.to_le_bytes());
        if aes {
            m.update(b"sAlT");
        }
        let h = m.finish();
        h[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn apply(&self, method: Method, id: ObjRef, data: &[u8]) -> Vec<u8> {
        match method {
            Method::None => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(id, false), data),
            Method::Aes128 | Method::Aes256 => {
                let key = self.object_key(id, true);
                aes_cbc_decrypt(&key, data)
            }
        }
    }

    pub fn string(&self, id: ObjRef, data: &[u8]) -> Vec<u8> {
        self.apply(self.strings, id, data)
    }

    pub fn stream(&self, id: ObjRef, data: &[u8]) -> Vec<u8> {
        self.apply(self.streams, id, data)
    }

    /// Decrypts every string inside `o`, the object `id`.
    pub fn object(&self, id: ObjRef, o: Object) -> Object {
        if self.strings == Method::None || Some(id) == self.encrypt_ref {
            return o;
        }
        self.walk(id, o, 0)
    }

    fn walk(&self, id: ObjRef, o: Object, depth: usize) -> Object {
        if depth > 64 {
            return o;
        }
        match o {
            Object::String(s) => Object::string(&self.string(id, &s)),
            Object::Array(a) => Object::array(a.iter().cloned().map(|x| self.walk(id, x, depth + 1)).collect()),
            Object::Dict(d) => Object::dict(self.walk_dict(id, &d, depth)),
            Object::Stream(s) => {
                let mut st = (*s).clone();
                st.dict = self.walk_dict(id, &s.dict, depth);
                Object::Stream(std::sync::Arc::new(st))
            }
            other => other,
        }
    }

    fn walk_dict(&self, id: ObjRef, d: &Dict, depth: usize) -> Dict {
        Dict { entries: d.entries.iter().map(|(k, v)| (k.clone(), self.walk(id, v.clone(), depth + 1))).collect() }
    }
}

fn methods(enc: &Dict, v: i64) -> (Method, Method) {
    if v < 4 {
        return (Method::Rc4, Method::Rc4);
    }
    let cf = enc.get("CF").and_then(Object::as_dict);
    let pick = |key: &str| -> Method {
        let name = enc.name(key).unwrap_or("Identity");
        if name == "Identity" {
            return Method::None;
        }
        let cfm = cf.and_then(|c| c.get(name)).and_then(Object::as_dict).and_then(|d| d.name("CFM")).unwrap_or("None");
        match cfm {
            "V2" => Method::Rc4,
            "AESV2" => Method::Aes128,
            "AESV3" => Method::Aes256,
            _ => Method::None,
        }
    };
    (pick("StrF"), pick("StmF"))
}

fn padded(pw: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

/// Algorithm 2: the file key from a user password.
fn key_r4(pw: &[u8], r: i64, len: usize, o: &[u8], p: i32, id0: &[u8], meta: bool) -> Vec<u8> {
    let mut m = Md5::new();
    m.update(&padded(pw));
    m.update(&o[..o.len().min(32)]);
    m.update(&p.to_le_bytes());
    m.update(id0);
    if r >= 4 && !meta {
        m.update(&[0xff; 4]);
    }
    let mut h = m.finish();
    if r >= 3 {
        for _ in 0..50 {
            h = md5(&h[..len]);
        }
    }
    h[..len].to_vec()
}

/// Algorithms 4 and 5: whether `key` is right, by recomputing `/U`.
fn check_user(key: &[u8], r: i64, u: &[u8], id0: &[u8]) -> bool {
    if r == 2 {
        return u.len() >= 32 && rc4(key, &PAD) == u[..32];
    }
    let mut m = Md5::new();
    m.update(&PAD);
    m.update(id0);
    let mut x = rc4(key, &m.finish());
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        x = rc4(&k, &x);
    }
    u.len() >= 16 && x[..16] == u[..16]
}

/// Algorithm 7: the user password hidden in `/O`, given the owner's.
fn owner_to_user(owner: &[u8], r: i64, len: usize, o: &[u8]) -> Vec<u8> {
    let mut h = md5(&padded(owner));
    if r >= 3 {
        for _ in 0..50 {
            h = md5(&h);
        }
    }
    let key = &h[..len];
    let o = &o[..o.len().min(32)];
    if r == 2 {
        return rc4(key, o);
    }
    let mut x = o.to_vec();
    for i in (0..=19u8).rev() {
        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        x = rc4(&k, &x);
    }
    x
}

/// Revisions 5 and 6: the 32-byte file key from the empty password, as
/// the user and then as the owner.
fn key_r6(r: i64, o: &[u8], u: &[u8], oe: &[u8], ue: &[u8]) -> Option<[u8; 32]> {
    if u.len() < 48 || ue.len() < 32 {
        return None;
    }
    let pw: &[u8] = b"";
    let hash = |pw: &[u8], salt: &[u8], extra: &[u8]| -> [u8; 32] {
        if r == 5 {
            let mut h = Sha256::new();
            h.update(pw);
            h.update(salt);
            h.update(extra);
            let d = h.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&d[..32]);
            out
        } else {
            hash_r6(pw, salt, extra)
        }
    };
    if hash(pw, &u[32..40], &[]) == u[..32] {
        let k = hash(pw, &u[40..48], &[]);
        return unwrap_key(&k, &ue[..32]);
    }
    if o.len() >= 48 && oe.len() >= 32 && hash(pw, &o[32..40], &u[..48]) == o[..32] {
        let k = hash(pw, &o[40..48], &u[..48]);
        return unwrap_key(&k, &oe[..32]);
    }
    None
}

fn unwrap_key(k: &[u8; 32], wrapped: &[u8]) -> Option<[u8; 32]> {
    let aes = Aes::new(k)?;
    let mut prev = [0u8; 16];
    let mut out = [0u8; 32];
    for (i, block) in wrapped.as_chunks::<16>().0.iter().take(2).enumerate() {
        let mut b = [0u8; 16];
        b.copy_from_slice(block);
        let plain = aes.decrypt_block(&b);
        for j in 0..16 {
            out[i * 16 + j] = plain[j] ^ prev[j];
        }
        prev = b;
    }
    Some(out)
}

/// Algorithm 2.B (ISO 32000-2).
fn hash_r6(pw: &[u8], salt: &[u8], udata: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(pw);
    h.update(salt);
    h.update(udata);
    let mut k: Vec<u8> = h.finalize().to_vec();
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (pw.len() + k.len() + udata.len()));
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(udata);
        }
        // The AES key and IV are the two halves of the previous hash.
        let key: [u8; 16] = std::array::from_fn(|i| k[i]);
        let iv: [u8; 16] = std::array::from_fn(|i| k[16 + i]);
        let e = aes_cbc_encrypt_raw(&key, &iv, &k1);
        let sum: u32 = e[..16].iter().map(|&b| u32::from(b)).sum();
        k = match sum % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        let last = u32::from(*e.last().unwrap_or(&0));
        if round >= 64 && last <= round - 32 {
            break;
        }
        if round > 2048 {
            break;
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    out
}

// ---- RC4 ------------------------------------------------------------------

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

// ---- MD5 (RFC 1321) -------------------------------------------------------

pub struct Md5 {
    state: [u32; 4],
    buf: Vec<u8>,
    len: u64,
}

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
    21,
];

fn md5_k(i: usize) -> u32 {
    // floor(|sin(i + 1)| · 2^32), computed once per call: exact in f64.
    ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32
}

impl Md5 {
    pub fn new() -> Md5 {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], buf: Vec::with_capacity(64), len: 0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        let mut data = data;
        if !self.buf.is_empty() {
            let need = 64 - self.buf.len();
            let take = need.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let block: [u8; 64] = self.buf[..].try_into().unwrap_or([0; 64]);
                self.block(&block);
                self.buf.clear();
            }
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            self.block(block);
        }
        self.buf.extend_from_slice(rest);
    }

    fn block(&mut self, b: &[u8; 64]) {
        let m: [u32; 16] =
            std::array::from_fn(|i| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]));
        let [mut a, mut bb, mut c, mut d] = self.state;
        for (i, &shift) in MD5_S.iter().enumerate() {
            let (f, g) = match i / 16 {
                0 => ((bb & c) | (!bb & d), i),
                1 => ((d & bb) | (!d & c), (5 * i + 1) % 16),
                2 => (bb ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (bb | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = bb;
            bb = bb.wrapping_add(a.wrapping_add(f).wrapping_add(md5_k(i)).wrapping_add(m[g]).rotate_left(shift));
            a = t;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(bb);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }

    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        let mut tail = vec![0x80u8];
        while (self.buf.len() + tail.len()) % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_le_bytes());
        // `update` would count the padding into the length; feed blocks directly.
        let mut all = std::mem::take(&mut self.buf);
        all.extend_from_slice(&tail);
        for c in all.as_chunks::<64>().0.iter() {
            self.block(c);
        }
        let state = self.state;
        std::array::from_fn(|i| state[i / 4].to_le_bytes()[i % 4])
    }
}

impl Default for Md5 {
    fn default() -> Md5 {
        Md5::new()
    }
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut m = Md5::new();
    m.update(data);
    m.finish()
}

// ---- AES (FIPS 197) -------------------------------------------------------

struct Tables {
    sbox: [u8; 256],
    inv: [u8; 256],
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut sbox = [0u8; 256];
        let mut inv = [0u8; 256];
        // Walk the multiplicative group with generator 3 and its inverse.
        let (mut p, mut q) = (1u8, 1u8);
        loop {
            p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1b } else { 0 };
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4) ^ 0x63;
            sbox[p as usize] = x;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        for (i, &s) in sbox.iter().enumerate() {
            inv[s as usize] = i as u8;
        }
        Tables { sbox, inv }
    })
}

fn xtime(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1b } else { 0 }
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

pub struct Aes {
    rounds: usize,
    keys: Vec<[u8; 16]>,
}

impl Aes {
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 => 4,
            24 => 6,
            32 => 8,
            _ => return None,
        };
        let rounds = nk + 6;
        let t = tables();
        let total = 4 * (rounds + 1);
        let mut w: Vec<[u8; 4]> = key.as_chunks::<4>().0.iter().map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut rcon = 1u8;
        for i in nk..total {
            let mut tmp = w[i - 1];
            if i % nk == 0 {
                tmp = [
                    t.sbox[tmp[1] as usize] ^ rcon,
                    t.sbox[tmp[2] as usize],
                    t.sbox[tmp[3] as usize],
                    t.sbox[tmp[0] as usize],
                ];
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                tmp = tmp.map(|b| t.sbox[b as usize]);
            }
            let prev = w[i - nk];
            w.push([prev[0] ^ tmp[0], prev[1] ^ tmp[1], prev[2] ^ tmp[2], prev[3] ^ tmp[3]]);
        }
        let keys = w
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let mut k = [0u8; 16];
                for (j, word) in c.iter().enumerate() {
                    k[j * 4..j * 4 + 4].copy_from_slice(word);
                }
                k
            })
            .collect();
        Some(Aes { rounds, keys })
    }

    pub fn encrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let t = tables();
        let mut s = *input;
        xor(&mut s, &self.keys[0]);
        for r in 1..=self.rounds {
            for b in &mut s {
                *b = t.sbox[*b as usize];
            }
            shift_rows(&mut s);
            if r != self.rounds {
                for c in 0..4 {
                    let col = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
                    for i in 0..4 {
                        s[c * 4 + i] =
                            gmul(col[i], 2) ^ gmul(col[(i + 1) % 4], 3) ^ col[(i + 2) % 4] ^ col[(i + 3) % 4];
                    }
                }
            }
            xor(&mut s, &self.keys[r]);
        }
        s
    }

    pub fn decrypt_block(&self, input: &[u8; 16]) -> [u8; 16] {
        let t = tables();
        let mut s = *input;
        xor(&mut s, &self.keys[self.rounds]);
        for r in (0..self.rounds).rev() {
            inv_shift_rows(&mut s);
            for b in &mut s {
                *b = t.inv[*b as usize];
            }
            xor(&mut s, &self.keys[r]);
            if r != 0 {
                for c in 0..4 {
                    let col = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
                    for i in 0..4 {
                        s[c * 4 + i] = gmul(col[i], 14)
                            ^ gmul(col[(i + 1) % 4], 11)
                            ^ gmul(col[(i + 2) % 4], 13)
                            ^ gmul(col[(i + 3) % 4], 9);
                    }
                }
            }
        }
        s
    }
}

fn xor(s: &mut [u8; 16], k: &[u8; 16]) {
    for i in 0..16 {
        s[i] ^= k[i];
    }
}

fn shift_rows(s: &mut [u8; 16]) {
    let c = *s;
    for col in 0..4 {
        for row in 0..4 {
            s[col * 4 + row] = c[((col + row) % 4) * 4 + row];
        }
    }
}

fn inv_shift_rows(s: &mut [u8; 16]) {
    let c = *s;
    for col in 0..4 {
        for row in 0..4 {
            s[((col + row) % 4) * 4 + row] = c[col * 4 + row];
        }
    }
}

/// AES-CBC as PDF stores it: a 16-byte IV, then the blocks, PKCS #7
/// padding removed when it is well formed.
pub fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let Some(aes) = Aes::new(key) else { return Vec::new() };
    if data.len() < 32 {
        return Vec::new();
    }
    let mut prev = [0u8; 16];
    prev.copy_from_slice(&data[..16]);
    let mut out = Vec::with_capacity(data.len() - 16);
    for c in data[16..].as_chunks::<16>().0.iter() {
        let mut b = [0u8; 16];
        b.copy_from_slice(c);
        let p = aes.decrypt_block(&b);
        for i in 0..16 {
            out.push(p[i] ^ prev[i]);
        }
        prev = b;
    }
    if let Some(&n) = out.last()
        && (1..=16).contains(&n)
        && out.len() >= n as usize
        && out[out.len() - n as usize..].iter().all(|&b| b == n)
    {
        out.truncate(out.len() - n as usize);
    }
    out
}

fn aes_cbc_encrypt_raw(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let Some(aes) = Aes::new(key) else { return Vec::new() };
    let mut prev = *iv;
    let mut out = Vec::with_capacity(data.len());
    for c in data.as_chunks::<16>().0.iter() {
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = c[i] ^ prev[i];
        }
        prev = aes.encrypt_block(&b);
        out.extend_from_slice(&prev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn md5_vectors() {
        assert_eq!(md5(b"").to_vec(), hex("d41d8cd98f00b204e9800998ecf8427e"));
        assert_eq!(md5(b"abc").to_vec(), hex("900150983cd24fb0d6963f7d28e17f72"));
        let long = b"12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(md5(long).to_vec(), hex("57edf4a22be3c955ac49da2e2107b67a"));
        let mut m = Md5::new();
        m.update(&long[..13]);
        m.update(&long[13..]);
        assert_eq!(m.finish(), md5(long));
    }

    #[test]
    fn rc4_vector() {
        assert_eq!(rc4(b"Key", b"Plaintext"), hex("bbf316e8d940af0ad3"));
    }

    #[test]
    fn aes_vectors() {
        // FIPS 197 appendix C.
        let pt: [u8; 16] = hex("00112233445566778899aabbccddeeff").try_into().unwrap();
        let a = Aes::new(&hex("000102030405060708090a0b0c0d0e0f")).unwrap();
        let ct = a.encrypt_block(&pt);
        assert_eq!(ct.to_vec(), hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
        assert_eq!(a.decrypt_block(&ct), pt);
        let a = Aes::new(&hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")).unwrap();
        let ct = a.encrypt_block(&pt);
        assert_eq!(ct.to_vec(), hex("8ea2b7ca516745bfeafc49904b496089"));
        assert_eq!(a.decrypt_block(&ct), pt);
    }

    #[test]
    fn cbc_round_trip_strips_padding() {
        // Key and IV derived at run time, like a document's would be.
        let key = md5(b"round-trip key");
        let iv = md5(b"round-trip iv");
        let mut plain = b"hello, encrypted world".to_vec();
        let pad = 16 - plain.len() % 16;
        plain.extend(std::iter::repeat_n(pad as u8, pad));
        let mut data = iv.to_vec();
        data.extend(aes_cbc_encrypt_raw(&key, &iv, &plain));
        assert_eq!(aes_cbc_decrypt(&key, &data), b"hello, encrypted world");
        assert!(aes_cbc_decrypt(&key, &data[..10]).is_empty());
    }

    #[test]
    fn rc4_document_with_empty_user_password() {
        // Build /O and /U the way a writer does (R3, 128-bit), then open.
        let id0 = b"0123456789abcdef";
        let owner = b"secret";
        let r = 3;
        let len = 16;
        // Algorithm 3: /O from the owner and (empty) user password.
        let mut h = md5(&padded(owner));
        for _ in 0..50 {
            h = md5(&h);
        }
        let mut o = rc4(&h[..len], &padded(b""));
        for i in 1..=19u8 {
            let k: Vec<u8> = h[..len].iter().map(|b| b ^ i).collect();
            o = rc4(&k, &o);
        }
        let p = -44i32;
        let key = key_r4(b"", r, len, &o, p, id0, true);
        // Algorithm 5: /U.
        let mut m = Md5::new();
        m.update(&PAD);
        m.update(id0);
        let mut u = rc4(&key, &m.finish());
        for i in 1..=19u8 {
            let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            u = rc4(&k, &u);
        }
        u.extend_from_slice(&[0u8; 16]);
        let mut enc = Dict::new();
        enc.insert("Filter", Object::name("Standard"));
        enc.insert("V", Object::Int(2));
        enc.insert("R", Object::Int(3));
        enc.insert("Length", Object::Int(128));
        enc.insert("O", Object::string(&o));
        enc.insert("U", Object::string(&u));
        enc.insert("P", Object::Int(i64::from(p)));
        let d = Decryptor::open(&enc, id0, None).unwrap();
        let id = ObjRef { num: 4, generation: 0 };
        let secret = rc4(&d.object_key(id, false), b"Hello");
        assert_eq!(d.string(id, &secret), b"Hello");
        // A non-empty user password is refused with the typed error.
        enc.insert("U", Object::string(&[0u8; 32]));
        assert_eq!(Decryptor::open(&enc, id0, None).unwrap_err().kind, crate::error::ErrorKind::Encrypted);
    }
}
