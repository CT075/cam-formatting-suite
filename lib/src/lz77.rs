use std::{collections::HashMap, ops::Range};

// GBA format taken from
//   http://florian.nouwt.com/wiki/index.php/LZ77_(Compression_Format)
// (now defunct, reproduced below)
//
// (Cam's notes: 0 = least-significant)
//
// Header:
//   | Offset | Length | Type    | Description                       |
//   -----------------------------------------------------------------
//   | 0x00   | 0x01   | Byte    | Compression type (0x10 for LZ77)  |
//   | 0x01   | 0x03   | UInt24  | Uncompressed Size                 |
//   | Compressed Data follows                                       |
//
// Compressed Data:
//
// The compressed data is divided in blocks. Each 8 blocks, there is a block
// header, indicating if blocks are compressed, or not.
//
// Block header:
//   | Bits   | Description                                          |
//   -----------------------------------------------------------------
//   | 0-7    | Subblock Type Flags (MSB = block 0, LSB = block 7)   |
//
// Block type 0 (Uncompressed):
//   | Bits   | Description                                          |
//   -----------------------------------------------------------------
//   | 0-7    | Byte to copy                                         |
//
// Block type 1 (Compressed):
//   Copies n+3 bytes from dst-offset-1 to dst.
//
//   | Bits   | Description                                          |
//   -----------------------------------------------------------------
//   | 0-11   | Backwards offset = offset                            |
//   | 12-15  | Data length = n                                      |
//
//   Note: The above two bytes need to be read in big endian.
//
//   (Cam's notes: The reference says that only bits 0-7 are used for [offset],
//   but every reference implementation I can find uses all 12 low bits. I have
//   have amended the table above in light of this.)

// 15
static MAX_REFERENCE_LENGTH: usize = 0b1111;

// 4095
static LOOKBACK_SIZE: usize = 0x0FFF;

static MAX_PREFIX_TABLE_SIZE: usize = 0x1000;

#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq, Eq)]
enum Block {
    Literal(u8),
    Reference { offset: usize, length: usize },
}

impl Block {
    fn size(&self) -> usize {
        match self {
            Self::Literal(_) => 1,
            Self::Reference { offset: _, length } => *length,
        }
    }

    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Literal(lit) => vec![*lit],
            Self::Reference { offset, length } => {
                usize::to_be_bytes(((*length - 3) << 12) | *offset)[6..8]
                    .to_vec()
            }
        }
    }
}

/// In layman's terms, this option controls how hard the compressor should be
/// trying.
///
/// LZ77 relies on identifying "runs" of data that are repeated within the
/// input data, and better compression can be achieved by looking for longer
/// runs. In exchange, it costs speed and memory to check every possibility, so
/// this option is exposed for the user to decide.
#[derive(Debug, Copy, Clone)]
pub enum CompressionStrategy {
    /// The "fast" option.
    CheckMostRecentOnly,
    /// The "best compression" option.
    CheckAllCandidates,
}

fn compress_abstract<'a>(
    raw: &'a [u8],
    strat: CompressionStrategy,
) -> impl Iterator<Item = Block> + 'a {
    Compressor::new(raw, strat)
}

pub fn compress(buf: &[u8], strategy: CompressionStrategy) -> Vec<u8> {
    let mut result = Vec::new();

    result.push(0x10);
    result.extend(&buf.len().to_le_bytes()[..3]);

    let mut blocks = compress_abstract(buf, strategy);

    while let Some(subblock0) = blocks.next() {
        // To avoid allocating an intermediate vec to hold all the subblocks,
        // we destructively modify the existing block header in-place as we
        // see said subblocks.
        let idx_of_block_header = result.len();
        let block_header = if matches!(subblock0, Block::Reference { .. }) {
            1 << 7
        } else {
            0
        };

        result.push(block_header);
        result.extend(subblock0.encode());

        for i in 1..8 {
            let subblock = match blocks.next() {
                None => break,
                Some(b) => b,
            };
            result.extend(subblock.encode());
            result[idx_of_block_header] |=
                if matches!(subblock, Block::Reference { .. }) {
                    1 << (8 - i - 1)
                } else {
                    0
                };
        }
    }

    result
}

pub trait BadBlockErrorHandler {
    fn bad_reference(block_num: usize, offs: usize) -> Self;
}

pub trait DecompressErrorHandler: BadBlockErrorHandler
where
    Self: Sized,
{
    fn data_too_short() -> Self;

    fn bad_header() -> Self;

    fn unexpected_eof(expected: &'static str) -> Self;
}

fn decompress_abstract_impl<E>(
    indexed_blocks: impl Iterator<Item = (usize, Block)>,
    out: &mut Vec<u8>,
    errs: &mut Vec<E>,
) where
    E: BadBlockErrorHandler,
{
    for (i, block) in indexed_blocks {
        match block {
            Block::Literal(lit) => out.push(lit),
            Block::Reference { offset, length } => {
                if out.len() < offset {
                    errs.push(E::bad_reference(i, offset));
                } else {
                    // TODO: elide bounds checks
                    for _ in 0..length {
                        out.push(out[out.len() - offset - 1])
                    }
                }
            }
        }
    }
}

fn decompress_block_stream<I, E>(
    source: I,
    inflated_size: usize,
) -> Decompressor<impl Iterator<Item = (usize, u8)>, E>
where
    I: Iterator<Item = u8>,
    E: DecompressErrorHandler,
{
    Decompressor::<_, E>::new(source.enumerate(), inflated_size)
}

pub fn decompress<E>(buf: &[u8]) -> (Vec<u8>, Vec<E>)
where
    E: DecompressErrorHandler,
{
    if buf.len() < 4 {
        return (vec![], vec![E::data_too_short()]);
    }

    let mut errs = Vec::new();

    if buf[0] != 0x10 {
        errs.push(E::bad_header());
    }

    let size =
        buf[1] as usize | ((buf[2] as usize) << 8) | ((buf[3] as usize) << 16);

    let mut out = Vec::with_capacity(size);

    let mut decompressor =
        decompress_block_stream::<_, E>(buf[4..].iter().map(|x| *x), size);

    decompress_abstract_impl(&mut decompressor, &mut out, &mut errs);

    errs.extend(decompressor.errors());

    (out, errs)
}

struct Compressor<'a> {
    raw: &'a [u8],
    cursor: usize,
    prefix_table: PrefixTable,
}

struct Match {
    idx: usize,
    length: usize,
}

impl<'a> Compressor<'a> {
    fn new(raw: &'a [u8], strategy: CompressionStrategy) -> Self {
        Self {
            raw,
            cursor: 0,
            prefix_table: PrefixTable::new(strategy),
        }
    }

    fn longest_prefix_from(&self, from: usize) -> usize {
        self.raw[from..]
            .iter()
            .take(MAX_REFERENCE_LENGTH - 3)
            .zip(&self.raw[self.cursor..])
            .take_while(|&(x, y)| x == y)
            .count()
    }

    fn best_match(&self, prefix: Prefix) -> Option<Match> {
        let candidates = self.prefix_table.candidates(prefix);

        candidates.and_then(|cs| {
            cs.into_iter()
                .filter(|&idx| self.cursor - idx <= LOOKBACK_SIZE)
                .fold(None, |acc, &idx| {
                    let length = self.longest_prefix_from(idx);

                    match acc {
                        None => Some(Match { idx, length }),
                        Some(m) => {
                            if length > m.length {
                                Some(Match { idx, length })
                            } else {
                                Some(m)
                            }
                        }
                    }
                })
        })
    }
}

impl<'a> Iterator for Compressor<'a> {
    type Item = Block;

    fn next(&mut self) -> Option<Self::Item> {
        enum Action {
            CheckTable(Prefix),
            EmitLiteral(u8),
        }

        // performs the bounds check only once
        let what_to_do: Action = match self.raw[self.cursor..] {
            [a, b, c, ..] => Action::CheckTable((a, b, c)),
            [a, _] | [a] => Action::EmitLiteral(a),
            [] => return None,
        };

        let (result, prefix_to_insert) = match what_to_do {
            Action::EmitLiteral(lit) => (Block::Literal(lit), None),
            Action::CheckTable(prefix @ (a, _, _)) => {
                match self.best_match(prefix) {
                    None => (Block::Literal(a), Some(prefix)),
                    Some(Match { idx, length }) => (
                        Block::Reference {
                            offset: self.cursor - idx - 1,
                            length,
                        },
                        Some(prefix),
                    ),
                }
            }
        };

        for &prefix in prefix_to_insert.iter() {
            self.prefix_table.insert_compress_if_necessary(
                prefix,
                self.cursor,
                self.cursor,
            )
        }

        self.cursor += result.size();

        Some(result)
    }
}

type Prefix = (u8, u8, u8);

enum PrefixTable {
    SingleStore(HashMap<Prefix, [usize; 1]>),
    AllStore(HashMap<Prefix, Vec<usize>>),
}

impl PrefixTable {
    fn new(strategy: CompressionStrategy) -> Self {
        match strategy {
            CompressionStrategy::CheckMostRecentOnly => {
                Self::SingleStore(HashMap::new())
            }
            CompressionStrategy::CheckAllCandidates => {
                Self::AllStore(HashMap::new())
            }
        }
    }

    // XXX: The case [Some([])] is impossible. This should be reflected in the
    // type.
    fn candidates<'a>(&'a self, prefix: Prefix) -> Option<&'a [usize]> {
        match self {
            Self::SingleStore(tbl) => tbl.get(&prefix).map(|v| &v[..]),
            Self::AllStore(tbl) => tbl.get(&prefix).map(|v| &v[..]),
        }
    }

    fn compress(&mut self, cursor: usize) {
        match self {
            Self::SingleStore(tbl) => {
                tbl.retain(|_, [idx]| cursor - *idx <= LOOKBACK_SIZE)
            }
            Self::AllStore(tbl) => {
                for (_, vs) in tbl.iter_mut() {
                    vs.retain(|idx| cursor - idx <= LOOKBACK_SIZE)
                }

                tbl.retain(|_, v| !v.is_empty())
            }
        }
    }

    fn insert(&mut self, prefix: Prefix, idx: usize) {
        match self {
            Self::SingleStore(tbl) => {
                tbl.insert(prefix, [idx]);
            }
            Self::AllStore(tbl) => {
                match tbl.get_mut(&prefix) {
                    Some(v) => {
                        v.push(idx);
                        return;
                    }
                    None => (),
                };

                tbl.insert(prefix, vec![idx]);
            }
        };
    }

    fn size(&self) -> usize {
        match self {
            Self::SingleStore(tbl) => tbl.len(),
            Self::AllStore(tbl) => {
                tbl.into_iter().fold(0, |acc, (_, v)| acc + v.len())
            }
        }
    }

    fn insert_compress_if_necessary(
        &mut self,
        prefix: Prefix,
        idx: usize,
        cursor: usize,
    ) {
        self.insert(prefix, idx);

        if self.size() > MAX_PREFIX_TABLE_SIZE {
            self.compress(cursor)
        }
    }
}

// XXX: This [E] type parameter is the actual worst, and requires turbofishing
// to thread it through everywhere it's needed. It'd be really nice to not need
// it, but I can't come up with any way to avoid specifying the type here.
struct Decompressor<I, E> {
    inflated_size: usize,
    seen: usize,
    block_header: Option<Biterator>,
    source: I,
    errors: Vec<E>,
}

impl<I, E> Decompressor<I, E>
where
    I: Iterator<Item = (usize, u8)>,
    E: DecompressErrorHandler,
{
    fn new(source: I, inflated_size: usize) -> Self {
        Self {
            source,
            inflated_size,
            seen: 0,
            block_header: None,
            errors: Vec::new(),
        }
    }

    fn errors(self) -> Vec<E> {
        self.errors
    }

    fn next_subblock_type(&mut self) -> Option<bool> {
        match &mut self.block_header {
            Some(biterator) => match biterator.next() {
                Some(b) => return Some(b),
                None => (),
            },
            None => (),
        };

        let mut biterator = Biterator::new(self.source.next()?.1);

        // This is an extra [Option] check that we might be able to avoid,
        // but it's honestly not worth it.
        let result = biterator.next().unwrap();

        self.block_header = Some(biterator);

        Some(result)
    }

    fn or_log_eof<T>(
        &mut self,
        x: Option<T>,
        expected: &'static str,
    ) -> Option<T> {
        match x {
            Some(x) => Some(x),
            None => {
                self.errors.push(E::unexpected_eof(expected));
                None
            }
        }
    }
}

// XXX: This representation loses granularity in errors as to why decompression
// failed. We can still report this to the user via debug messages, but it's
// not ideal.
//
// On the other hand, errors in this function are "fatal" (to the decompression
// itself, at least) anyway, so it might not matter.
impl<I, E> Iterator for Decompressor<I, E>
where
    I: Iterator<Item = (usize, u8)>,
    E: DecompressErrorHandler,
{
    // In order to produce accurate failure messages, we need to mark each
    // block with its location in the source buffer.
    type Item = (usize, Block);

    fn next(&mut self) -> Option<Self::Item> {
        if self.seen >= self.inflated_size {
            return None;
        }

        let next_subblock_is_compressed = {
            let next_type = self.next_subblock_type();
            self.or_log_eof(next_type, "expected [block_header] byte")?
        };

        if next_subblock_is_compressed {
            let (index, subblock1) = {
                let sb1 = self.source.next();
                self.or_log_eof(sb1, "expected reference byte")
                    .map(|(a, b)| (a, b as u16))?
            };
            let subblock2 = {
                let sb2 = self.source.next();
                self.or_log_eof(sb2, "references should be two bytes")?.1 as u16
            };
            let subblock: u16 = (subblock1 << 8) | subblock2;

            let length = ((subblock >> 12) + 3) as usize;
            let offset = (subblock & 0x0FFF) as usize;

            let result = Block::Reference { length, offset };
            self.seen += length;
            Some((index, result))
        } else {
            let result = {
                let lit = self.source.next();
                self.or_log_eof(lit, "expected literal byte")
                    .map(|(idx, lit)| (idx, Block::Literal(lit)))?
            };
            self.seen += 1;
            Some(result)
        }
    }
}

struct Biterator {
    raw: u8,
    bit_index: Range<usize>,
}

impl Biterator {
    fn new(raw: u8) -> Self {
        Self {
            raw,
            bit_index: 0..8,
        }
    }
}

impl Iterator for Biterator {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        self.bit_index
            .next()
            .map(|idx| (self.raw & (1 << (8 - idx - 1))) != 0)
    }
}
