// RIPTOR PATCH: BitTorrent Message Stream Encryption (MSE/PE). Obfuscation, not real security:
// RC4 and an unauthenticated Diffie-Hellman exchange, as the protocol defines them.

use std::{
    io::{self, IoSliceMut},
    pin::Pin,
    task::{Context, Poll, ready},
};

use anyhow::{Context as _, bail, ensure};
use librqbit_core::hash_id::Id20;
use num_bigint::BigUint;
use rand::RngExt;
use rc4::{KeyInit, StreamCipher, consts::U20};
use serde::{Deserialize, Serialize};
use sha1w::ISha1;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    type_aliases::{BoxAsyncReadVectored, BoxAsyncWrite},
    vectored_traits::AsyncReadVectored,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionPolicy {
    /// Plain connections only, as upstream librqbit.
    #[default]
    Disabled,
    /// Encrypt when the peer can; fall back to plain otherwise.
    Prefer,
    /// Only encrypted connections.
    Require,
}

const P: [u8; KEY_LEN] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xC9, 0x0F, 0xDA, 0xA2, 0x21, 0x68, 0xC2, 0x34,
    0xC4, 0xC6, 0x62, 0x8B, 0x80, 0xDC, 0x1C, 0xD1, 0x29, 0x02, 0x4E, 0x08, 0x8A, 0x67, 0xCC, 0x74,
    0x02, 0x0B, 0xBE, 0xA6, 0x3B, 0x13, 0x9B, 0x22, 0x51, 0x4A, 0x08, 0x79, 0x8E, 0x34, 0x04, 0xDD,
    0xEF, 0x95, 0x19, 0xB3, 0xCD, 0x3A, 0x43, 0x1B, 0x30, 0x2B, 0x0A, 0x6D, 0xF2, 0x5F, 0x14, 0x37,
    0x4F, 0xE1, 0x35, 0x6D, 0x6D, 0x51, 0xC2, 0x45, 0xE4, 0x85, 0xB5, 0x76, 0x62, 0x5E, 0x7E, 0xC6,
    0xF4, 0x4C, 0x42, 0xE9, 0xA6, 0x3A, 0x36, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0x05, 0x63,
];
const KEY_LEN: usize = 96;
const VC: [u8; 8] = [0; 8];
const CRYPTO_PLAIN: u32 = 0x01;
const CRYPTO_RC4: u32 = 0x02;
const MAX_PAD: usize = 512;
// The initial payload is normally just the 68-byte BitTorrent handshake.
const MAX_IA: usize = 16 * 1024;
const PLAIN_PREFIX: &[u8; 20] = b"\x13BitTorrent protocol";
const WRITE_CHUNK: usize = 64 * 1024;

type Cipher = rc4::Rc4<U20>;

fn sha1(parts: &[&[u8]]) -> [u8; 20] {
    let mut h = sha1w::Sha1::new();
    for p in parts {
        h.update(p);
    }
    h.finish()
}

/// RC4 with the first 1024 bytes of keystream thrown away, as the spec requires.
fn cipher(key: [u8; 20]) -> Cipher {
    let mut c = Cipher::new_from_slice(&key).expect("20-byte key");
    c.apply_keystream(&mut [0u8; 1024]);
    c
}

fn xor20(a: [u8; 20], b: [u8; 20]) -> [u8; 20] {
    std::array::from_fn(|i| a[i] ^ b[i])
}

fn random_pad() -> Vec<u8> {
    let mut rng = rand::rng();
    let mut pad = vec![0u8; rng.random_range(0..=MAX_PAD)];
    rand::fill(&mut pad[..]);
    pad
}

struct Dh {
    private: BigUint,
    public: [u8; KEY_LEN],
}

impl Dh {
    fn new() -> Self {
        let mut x = [0u8; 20];
        rand::fill(&mut x);
        let private = BigUint::from_bytes_be(&x);
        let public = to_key(&BigUint::from(2u32).modpow(&private, &BigUint::from_bytes_be(&P)));
        Self { private, public }
    }

    fn shared(&self, theirs: &[u8]) -> anyhow::Result<[u8; KEY_LEN]> {
        let p = BigUint::from_bytes_be(&P);
        let y = BigUint::from_bytes_be(theirs);
        // 0, 1 and p-1 would make the shared secret guessable.
        ensure!(
            y > BigUint::from(1u32) && y < &p - 1u32,
            "degenerate public key"
        );
        Ok(to_key(&y.modpow(&self.private, &p)))
    }
}

fn to_key(n: &BigUint) -> [u8; KEY_LEN] {
    let b = n.to_bytes_be();
    let mut out = [0u8; KEY_LEN];
    out[KEY_LEN - b.len()..].copy_from_slice(&b);
    out
}

/// Handshake bytes read so far, so nothing past the handshake is lost.
struct Inbox {
    reader: BoxAsyncReadVectored,
    buf: Vec<u8>,
}

impl Inbox {
    async fn read_more(&mut self) -> anyhow::Result<()> {
        let mut tmp = [0u8; 4096];
        let n = self.reader.read(&mut tmp).await?;
        ensure!(n > 0, "peer closed the connection during the encryption handshake");
        self.buf.extend_from_slice(&tmp[..n]);
        Ok(())
    }

    async fn fill(&mut self, n: usize) -> anyhow::Result<()> {
        while self.buf.len() < n {
            self.read_more().await?;
        }
        Ok(())
    }

    async fn take(&mut self, n: usize) -> anyhow::Result<Vec<u8>> {
        self.fill(n).await?;
        Ok(self.buf.drain(..n).collect())
    }

    /// Skips up to `MAX_PAD` bytes of padding until `marker`, and past it.
    async fn sync_to(&mut self, marker: &[u8]) -> anyhow::Result<()> {
        loop {
            if let Some(i) = self.buf.windows(marker.len()).position(|w| w == marker) {
                ensure!(i <= MAX_PAD, "encryption handshake padding too long");
                self.buf.drain(..i + marker.len());
                return Ok(());
            }
            ensure!(
                self.buf.len() < MAX_PAD + marker.len(),
                "encryption handshake never synchronised"
            );
            self.read_more().await?;
        }
    }
}

fn be16(b: &[u8]) -> usize {
    u16::from_be_bytes([b[0], b[1]]) as usize
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Our side of an outgoing connection. Asks for RC4 only.
pub(crate) async fn initiate(
    reader: BoxAsyncReadVectored,
    writer: BoxAsyncWrite,
    info_hash: Id20,
) -> anyhow::Result<(BoxAsyncReadVectored, BoxAsyncWrite)> {
    let tamper = testing::Tamper::default();
    let (r, w, _) = initiate_with(reader, writer, info_hash, CRYPTO_RC4, &[], &tamper).await?;
    Ok((r, w))
}

// `provide`, `ia` and `tamper` let the tests act like other clients, or like hostile ones.
async fn initiate_with(
    reader: BoxAsyncReadVectored,
    mut writer: BoxAsyncWrite,
    info_hash: Id20,
    provide: u32,
    ia: &[u8],
    tamper: &testing::Tamper,
) -> anyhow::Result<(BoxAsyncReadVectored, BoxAsyncWrite, u32)> {
    let skey = info_hash.0;
    let dh = Dh::new();
    let public = tamper.public_key.unwrap_or(dh.public);
    let pad = tamper.pad_a.map_or_else(random_pad, |n| vec![7u8; n]);
    writer.write_all(&[&public[..], &pad].concat()).await?;

    let mut inbox = Inbox {
        reader,
        buf: Vec::new(),
    };
    let theirs = inbox.take(KEY_LEN).await?;
    let s = dh.shared(&theirs)?;
    let mut enc = cipher(sha1(&[b"keyA", &s, &skey]));
    let mut dec = cipher(sha1(&[b"keyB", &s, &skey]));

    let mut tail = Vec::with_capacity(16 + ia.len());
    tail.extend_from_slice(&tamper.vc.unwrap_or(VC));
    tail.extend_from_slice(&provide.to_be_bytes());
    tail.extend_from_slice(&tamper.pad_c_len.unwrap_or(0).to_be_bytes());
    tail.extend_from_slice(&tamper.ia_len.unwrap_or(ia.len() as u16).to_be_bytes());
    tail.extend_from_slice(ia);
    enc.apply_keystream(&mut tail);
    let asked = tamper.info_hash.map_or(skey, |h| h.0);
    let msg = [
        &sha1(&[b"req1", &s])[..],
        &xor20(sha1(&[b"req2", &asked]), sha1(&[b"req3", &s])),
        &tail,
    ]
    .concat();
    writer.write_all(&msg).await?;

    // The peer's encrypted VC follows its padding; find it by what it encrypts to.
    let mut marker = VC;
    cipher(sha1(&[b"keyB", &s, &skey])).apply_keystream(&mut marker);
    inbox.sync_to(&marker).await?;
    dec.apply_keystream(&mut [0u8; 8]);

    let mut head = inbox.take(6).await?;
    dec.apply_keystream(&mut head);
    let select = be32(&head);
    let pad_d = be16(&head[4..]);
    ensure!(pad_d <= MAX_PAD, "encryption handshake padding too long");
    let mut pad = inbox.take(pad_d).await?;
    dec.apply_keystream(&mut pad);
    ensure!(
        select & provide != 0 && select.count_ones() == 1,
        "peer picked an encryption method we didn't offer"
    );

    let Inbox { reader, mut buf } = inbox;
    if select == CRYPTO_RC4 {
        dec.apply_keystream(&mut buf);
        let r = MseReader::new(reader, buf, Some(dec));
        Ok((Box::new(r), Box::new(MseWriter::new(writer, enc)), select))
    } else {
        Ok((Box::new(MseReader::new(reader, buf, None)), writer, select))
    }
}

/// The other side: a plain handshake passes through unless encryption is required.
/// Returns whether the stream is RC4-encrypted.
pub(crate) async fn accept(
    reader: BoxAsyncReadVectored,
    mut writer: BoxAsyncWrite,
    policy: EncryptionPolicy,
    info_hashes: impl FnOnce() -> Vec<Id20>,
) -> anyhow::Result<(BoxAsyncReadVectored, BoxAsyncWrite, bool)> {
    let mut inbox = Inbox {
        reader,
        buf: Vec::new(),
    };
    inbox.fill(PLAIN_PREFIX.len()).await?;
    if inbox.buf.starts_with(PLAIN_PREFIX) {
        ensure!(
            policy != EncryptionPolicy::Require,
            "peer connected without encryption, which is required"
        );
        let Inbox { reader, buf } = inbox;
        return Ok((Box::new(MseReader::new(reader, buf, None)), writer, false));
    }
    ensure!(
        policy != EncryptionPolicy::Disabled,
        "peer asked for encryption, which is off"
    );

    let theirs = inbox.take(KEY_LEN).await?;
    let dh = Dh::new();
    writer.write_all(&[&dh.public[..], &random_pad()].concat()).await?;
    let s = dh.shared(&theirs)?;

    inbox.sync_to(&sha1(&[b"req1", &s])).await?;
    let mixed: [u8; 20] = inbox.take(20).await?.try_into().unwrap();
    let wanted = xor20(mixed, sha1(&[b"req3", &s]));
    let skey = info_hashes()
        .into_iter()
        .find(|h| sha1(&[b"req2", &h.0]) == wanted)
        .context("encrypted peer asked for a torrent we don't have")?
        .0;
    let mut dec = cipher(sha1(&[b"keyA", &s, &skey]));
    let mut enc = cipher(sha1(&[b"keyB", &s, &skey]));

    let mut head = inbox.take(14).await?;
    dec.apply_keystream(&mut head);
    ensure!(head[..8] == VC, "bad verification constant");
    let provide = be32(&head[8..]);
    let pad_c = be16(&head[12..]);
    ensure!(pad_c <= MAX_PAD, "encryption handshake padding too long");
    let mut pad = inbox.take(pad_c).await?;
    dec.apply_keystream(&mut pad);
    let mut len = inbox.take(2).await?;
    dec.apply_keystream(&mut len);
    let ia_len = be16(&len);
    ensure!(ia_len <= MAX_IA, "initial payload too long");
    let mut ia = inbox.take(ia_len).await?;
    dec.apply_keystream(&mut ia);

    let select = if provide & CRYPTO_RC4 != 0 {
        CRYPTO_RC4
    } else if provide & CRYPTO_PLAIN != 0 && policy == EncryptionPolicy::Prefer {
        CRYPTO_PLAIN
    } else {
        bail!("peer offered no encryption method we accept");
    };
    let mut reply = Vec::with_capacity(14);
    reply.extend_from_slice(&VC);
    reply.extend_from_slice(&select.to_be_bytes());
    reply.extend_from_slice(&0u16.to_be_bytes());
    enc.apply_keystream(&mut reply);
    writer.write_all(&reply).await?;

    let Inbox { reader, mut buf } = inbox;
    if select == CRYPTO_RC4 {
        dec.apply_keystream(&mut buf);
        ia.extend_from_slice(&buf);
        let r = MseReader::new(reader, ia, Some(dec));
        Ok((Box::new(r), Box::new(MseWriter::new(writer, enc)), true))
    } else {
        ia.extend_from_slice(&buf);
        Ok((Box::new(MseReader::new(reader, ia, None)), writer, false))
    }
}

/// Bytes already read (and decrypted) come first, then the decrypted stream.
struct MseReader {
    inner: BoxAsyncReadVectored,
    pending: Vec<u8>,
    pos: usize,
    cipher: Option<Cipher>,
}

impl MseReader {
    fn new(inner: BoxAsyncReadVectored, pending: Vec<u8>, cipher: Option<Cipher>) -> Self {
        Self {
            inner,
            pending,
            pos: 0,
            cipher,
        }
    }

    fn take_pending(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.pending.len() - self.pos);
        out[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        if self.pos == self.pending.len() {
            self.pending = Vec::new();
            self.pos = 0;
        }
        n
    }
}

impl AsyncRead for MseReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.pending.len() {
            let n = this.take_pending(buf.initialize_unfilled());
            buf.advance(n);
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(cx, buf))?;
        if let Some(c) = &mut this.cipher {
            c.apply_keystream(&mut buf.filled_mut()[before..]);
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncReadVectored for MseReader {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.pos < this.pending.len() {
            let mut total = 0;
            for s in vec.iter_mut() {
                if this.pending.is_empty() {
                    break;
                }
                total += this.take_pending(s);
            }
            return Poll::Ready(Ok(total));
        }
        let n = ready!(Pin::new(&mut this.inner).poll_read_vectored(cx, vec))?;
        if let Some(c) = &mut this.cipher {
            let mut left = n;
            for s in vec.iter_mut() {
                let k = left.min(s.len());
                c.apply_keystream(&mut s[..k]);
                left -= k;
                if left == 0 {
                    break;
                }
            }
        }
        Poll::Ready(Ok(n))
    }
}

/// Reports a write only once its encrypted bytes are on the wire: nothing here ever flushes,
/// and the RC4 stream must match what the peer receives.
struct MseWriter {
    inner: BoxAsyncWrite,
    cipher: Cipher,
    pending: Vec<u8>,
    pos: usize,
    accepted: usize,
}

impl MseWriter {
    fn new(inner: BoxAsyncWrite, cipher: Cipher) -> Self {
        Self {
            inner,
            cipher,
            pending: Vec::new(),
            pos: 0,
            accepted: 0,
        }
    }

    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.pos < self.pending.len() {
            let n = ready!(Pin::new(&mut self.inner).poll_write(cx, &self.pending[self.pos..]))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.pos += n;
        }
        self.pending.clear();
        self.pos = 0;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for MseWriter {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.pending.is_empty() {
            if data.is_empty() {
                return Poll::Ready(Ok(0));
            }
            // A retry after Pending finds these bytes still here and finishes them first.
            let n = data.len().min(WRITE_CHUNK);
            this.pending.extend_from_slice(&data[..n]);
            this.cipher.apply_keystream(&mut this.pending);
            this.accepted = n;
        }
        ready!(this.poll_drain(cx))?;
        Poll::Ready(Ok(std::mem::take(&mut this.accepted)))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_drain(cx))?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_drain(cx))?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

/// Handshake entry points for Riptor's tests, over loopback TCP.
#[doc(hidden)]
pub mod testing {
    use super::*;

    pub type Reader = Box<dyn AsyncRead + Send + Unpin>;
    pub type Writer = Box<dyn AsyncWrite + Send + Unpin>;

    /// Ways a hostile initiator can break the handshake. Real connections never set any.
    #[derive(Clone, Debug, Default)]
    pub struct Tamper {
        pub public_key: Option<[u8; KEY_LEN]>,
        pub pad_a: Option<usize>,
        pub vc: Option<[u8; 8]>,
        pub pad_c_len: Option<u16>,
        pub ia_len: Option<u16>,
        /// Asks for this torrent instead.
        pub info_hash: Option<Id20>,
    }

    pub const PRIME: [u8; KEY_LEN] = P;

    /// The handshake as a hostile client would run it.
    pub async fn initiate_tampered(
        stream: tokio::net::TcpStream,
        info_hash: Id20,
        tamper: Tamper,
    ) -> anyhow::Result<()> {
        let (r, w) = split(stream);
        initiate_with(r, w, info_hash, CRYPTO_RC4, &[], &tamper).await?;
        Ok(())
    }

    fn split(stream: tokio::net::TcpStream) -> (BoxAsyncReadVectored, BoxAsyncWrite) {
        let (r, w) = stream.into_split();
        (Box::new(r), Box::new(w))
    }

    /// Offers RC4 and/or plaintext; `ia` is sent as the initial payload.
    pub async fn initiate(
        stream: tokio::net::TcpStream,
        info_hash: Id20,
        offer_rc4: bool,
        offer_plain: bool,
        ia: &[u8],
    ) -> anyhow::Result<(Reader, Writer, bool)> {
        let (r, w) = split(stream);
        let provide = if offer_rc4 { CRYPTO_RC4 } else { 0 } | if offer_plain { CRYPTO_PLAIN } else { 0 };
        let (r, w, select) =
            initiate_with(r, w, info_hash, provide, ia, &Tamper::default()).await?;
        let r: Reader = r;
        let w: Writer = w;
        Ok((r, w, select == CRYPTO_RC4))
    }

    pub async fn accept(
        stream: tokio::net::TcpStream,
        policy: EncryptionPolicy,
        info_hashes: Vec<Id20>,
    ) -> anyhow::Result<(Reader, Writer, bool)> {
        let (r, w) = split(stream);
        let (r, w, encrypted) = super::accept(r, w, policy, || info_hashes).await?;
        let r: Reader = r;
        let w: Writer = w;
        Ok((r, w, encrypted))
    }

    /// Accepts on `stream` the way an incoming connection is, then reads the plain BitTorrent
    /// handshake through the same buffer the session uses.
    pub async fn accept_handshake(
        stream: tokio::net::TcpStream,
        policy: EncryptionPolicy,
        info_hashes: Vec<Id20>,
    ) -> anyhow::Result<(Id20, bool)> {
        let (r, w) = split(stream);
        let (mut r, _w, encrypted) = super::accept(r, w, policy, || info_hashes).await?;
        let mut buf = crate::read_buf::ReadBuf::new();
        let h = buf
            .read_handshake(&mut r, std::time::Duration::from_secs(10))
            .await?;
        Ok((h.info_hash, encrypted))
    }
}
