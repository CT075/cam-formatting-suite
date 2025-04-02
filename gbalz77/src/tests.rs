use quickcheck::{Arbitrary, Gen};
use quickcheck_macros::quickcheck;

use super::*;

#[derive(Debug, PartialEq, Eq)]
enum E {
    BadReference(usize, usize),
    UnexpectedEof(&'static str),
    DataTooShort,
    BadHeader,
}

impl BadBlockErrorHandler for E {
    fn bad_reference(block_num: usize, offs: usize) -> Self {
        Self::BadReference(block_num, offs)
    }
}

impl DecompressErrorHandler for E {
    fn data_too_short() -> Self {
        Self::DataTooShort
    }

    fn bad_header() -> Self {
        Self::BadHeader
    }

    fn unexpected_eof(s: &'static str) -> Self {
        Self::UnexpectedEof(s)
    }
}

fn decompress_abstract(
    indexed_blocks: impl Iterator<Item = (usize, Block)>,
) -> (Vec<u8>, Vec<E>) {
    let mut out = Vec::new();
    let mut errs = Vec::new();

    decompress_abstract_impl(indexed_blocks, &mut out, &mut errs);

    (out, errs)
}

#[test]
fn basic_compression_abstract() {
    let src: &[u8] = "Blahblahblahblah".as_bytes();

    use Block::*;
    assert_eq!(
        compress_abstract(src, CompressionStrategy::CheckAllCandidates)
            .collect::<Vec<_>>(),
        vec![
            Literal(66),
            Literal(108),
            Literal(97),
            Literal(104),
            Literal(98),
            Reference {
                offset: 3,
                length: 11
            }
        ]
    )
}

impl Arbitrary for CompressionStrategy {
    fn arbitrary(g: &mut Gen) -> Self {
        match bool::arbitrary(g) {
            true => Self::CheckMostRecentOnly,
            false => Self::CheckAllCandidates,
        }
    }
}

#[quickcheck]
fn quickcheck_decompress_compress_equals_id_abstract(
    raw: Vec<u8>,
    strategy: CompressionStrategy,
) -> bool {
    let (out, errs): (_, Vec<E>) =
        decompress_abstract(compress_abstract(&raw, strategy).enumerate());

    errs.is_empty() && out == raw
}

#[quickcheck]
fn quickcheck_decompress_compress_equals_id_concrete(
    raw: Vec<u8>,
    strategy: CompressionStrategy,
) -> bool {
    let (out, errs): (_, Vec<E>) = decompress(&compress(&raw, strategy));

    errs.is_empty() && out == raw
}
