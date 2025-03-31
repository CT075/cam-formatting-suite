// Generic GBA image manipulation library.
//
// TODO: Right now, we constrain `Subpixel = u8` in many cases. We should
// instead try to convert from wider depths by trimming the LSBs.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    io::Cursor,
    iter::FromIterator,
};

use image::{
    guess_format, GenericImageView, ImageFormat, ImageReader, Pixel, Rgb,
};
use itertools::Itertools;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("image has too many colors")]
    TooManyColors,
    #[error("image contains a color not in the provided palette")]
    UnknownColor,
    #[error("error processing image")]
    ImageError(#[from] image::ImageError),
    #[error("error processing png image")]
    PngError(#[from] png::DecodingError),
}

#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

impl<P> From<P> for Color
where
    P: Pixel<Subpixel = u8>,
{
    fn from(p: P) -> Self {
        let Rgb([r, g, b]) = p.to_rgb();
        Self::rgb(r, g, b)
    }
}

#[derive(Clone)]
pub struct Palette(Vec<Color>);

impl Palette {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    // TODO: more than 16 colors? multiple palette banks?
    pub fn validate(&self) -> Result<(), Error> {
        if self.0.len() > 16 {
            return Err(Error::TooManyColors);
        }

        Ok(())
    }
}

impl From<Vec<Color>> for Palette {
    fn from(p: Vec<Color>) -> Self {
        Self(p)
    }
}

impl FromIterator<Color> for Palette {
    fn from_iter<I>(colors: I) -> Self
    where
        I: IntoIterator<Item = Color>,
    {
        Self::from(colors.into_iter().collect::<Vec<_>>())
    }
}

pub struct GBAImage {
    palette: Palette,
    width: usize,
    height: usize,
    // INVARIANT: forall i . data[i] < palette.len()
    // INVARIANT: data.len() = width * height, row-major.
    data: Vec<usize>,
}

impl GBAImage {
    pub fn validate(&self) -> Result<(), Error> {
        self.palette.validate()?;
        Ok(())
    }

    pub fn from_image_view<V>(
        img: &V,
        colors: Option<Palette>,
    ) -> Result<Self, Error>
    where
        V: GenericImageView,
        V::Pixel: Pixel<Subpixel = u8>,
    {
        let (fixed_palette, mut colors) = match colors {
            None => (true, HashMap::new()),
            Some(colors) => (
                false,
                colors
                    .0
                    .iter()
                    .enumerate()
                    .map(|(x, i)| (*i, x))
                    .collect::<HashMap<Color, usize>>(),
            ),
        };
        let width = img.width() as usize;
        let height = img.height() as usize;
        let mut count = colors.len();

        let data = img
            .pixels()
            .map(|(_x, _y, pix)| {
                let color = Color::from(pix);

                match colors.get(&color) {
                    Some(idx) => Ok(*idx),
                    None => {
                        if fixed_palette {
                            Err(Error::UnknownColor)
                        } else {
                            let idx = count;
                            colors.insert(color, idx);
                            count += 1;
                            Ok(idx)
                        }
                    }
                }
            })
            .collect::<Result<Vec<_>, Error>>()?;

        let palette = colors
            .into_iter()
            .sorted_by(|&(_c1, idx1), &(_c2, idx2)| idx1.cmp(&idx2))
            .map(|(c, _idx)| c)
            .collect();

        Ok(Self {
            palette,
            width,
            height,
            data,
        })
    }

    pub fn with_inferred_palette<V>(img: &V) -> Result<Self, Error>
    where
        V: GenericImageView,
        V::Pixel: Pixel<Subpixel = u8>,
    {
        Self::from_image_view(img, None)
    }

    pub fn with_known_palette<V>(
        img: &V,
        palette: Palette,
    ) -> Result<Self, Error>
    where
        V: GenericImageView,
        V::Pixel: Pixel<Subpixel = u8>,
    {
        Self::from_image_view(img, Some(palette))
    }
}

// TODO: more than 16 colors?
fn guess_fixed_palette<V>(img: &V) -> Option<Palette>
where
    V: GenericImageView,
    V::Pixel: Pixel<Subpixel = u8>,
{
    // If there are exactly 16 unique colors in the top left, use that as the
    // palette, in that order.
    let first16 = img
        .pixels()
        .take(16)
        .map(|(_x, _y, pxl)| Color::from(pxl))
        .collect::<Vec<_>>();

    if first16.iter().collect::<HashSet<_>>().len() == 16 {
        return Some(Palette::from(first16));
    }

    let topleft8x2 = (0..8)
        .cartesian_product(0..2)
        .map(|(x, y)| {
            if img.in_bounds(x, y) {
                unsafe { Some(Color::from(img.unsafe_get_pixel(x, y))) }
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>();

    let colors = match topleft8x2 {
        None => return None,
        Some(colors) => colors,
    };

    if colors.iter().collect::<HashSet<_>>().len() == 16 {
        return Some(Palette::from(colors));
    }

    None
}

fn load_png_palette(buf: &[u8]) -> Result<Option<Palette>, Error> {
    let decoder = png::Decoder::new(buf);
    let reader = decoder.read_info()?;

    if let Some(palette_bytes) = reader.info().palette.as_ref() {
        return Ok(Some(
            palette_bytes
                .iter()
                .tuples::<(_, _, _)>()
                .map(|(r, g, b)| Color::rgb(*r, *g, *b))
                .collect(),
        ));
    }

    Ok(None)
}

pub fn convert_image(
    buf: &[u8],
    format: Option<ImageFormat>,
    palette: Option<Palette>,
) -> Result<GBAImage, Error> {
    let format = match format {
        Some(format) => format,
        None => guess_format(buf)?,
    };

    let palette = if matches!(palette, None) {
        use ImageFormat::*;
        match format {
            Png => load_png_palette(buf)?,
            _ => None,
        }
    } else {
        palette
    };

    let mut reader = ImageReader::new(Cursor::new(buf));
    reader.set_format(format);
    let img = reader.decode()?;

    let palette = if matches!(palette, None) {
        guess_fixed_palette(&img)
    } else {
        palette
    };

    GBAImage::from_image_view(&img, palette)
}
