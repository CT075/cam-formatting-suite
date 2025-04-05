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
    // Errors that can come from trying to insert/format a bad image
    #[error("image has too many colors")]
    TooManyColors,
    #[error("image contains a color not in the provided palette")]
    UnknownColor,
    #[error("width and height must be multiples of 8")]
    BadDimensions,

    // Internal errors/bugs (raised by [validate])
    #[error("BUG: image dimensions don't match internal buffer")]
    DimensionMismatch,
    #[error("BUG: image contains color index >15")]
    BadColorIndex,

    // Errors from other libraries
    #[error("error processing image")]
    ImageError(#[from] image::ImageError),
    #[error("error processing png image")]
    PngError(#[from] png::DecodingError),
}

#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_16bit(p: u16) -> Self {
        const LOW5_MASK: u16 = 0b11111;
        let r = ((p & LOW5_MASK) as u8) << 3;
        let g = (((p >> 5) & LOW5_MASK) as u8) << 3;
        let b = (((p >> 10) & LOW5_MASK) as u8) << 3;
        Self::rgb(r, g, b)
    }

    pub fn from_le_bytes((l, r): (u8, u8)) -> Self {
        Self::from_16bit(((r as u16) << 8) | (l as u16))
    }

    pub fn to_16bit(self) -> u16 {
        (((self.b >> 3) as u16) << 10)
            | (((self.g >> 3) as u16) << 5)
            | ((self.r >> 3) as u16)
    }

    pub fn to_le_bytes(self) -> [u8; 2] {
        const LOW8_MASK: u16 = 0b11111111;
        let short = self.to_16bit();
        [(short & LOW8_MASK) as u8, ((short >> 8) & LOW8_MASK) as u8]
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        let [b1, b2] = self.to_le_bytes();
        write!(f, "{:02X}{:02X}", b1, b2)?;
        Ok(())
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

#[derive(Clone, Debug)]
pub struct Palette(Vec<Color>);

impl Palette {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn lookup(&self, idx: usize) -> Option<Color> {
        self.0.get(idx).copied()
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0
            .iter()
            .flat_map(|col| Color::to_le_bytes(*col))
            .collect()
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

impl FromIterator<u8> for Palette {
    fn from_iter<I>(colors: I) -> Self
    where
        I: IntoIterator<Item = u8>,
    {
        colors
            .into_iter()
            .tuples::<(_, _)>()
            .map(Color::from_le_bytes)
            .collect()
    }
}

impl std::fmt::Display for Palette {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        for color in self.0.iter() {
            write!(f, "{}", color)?;
        }
        Ok(())
    }
}

pub struct GBAImage {
    pub palette: Palette,
    pub width: usize,
    pub height: usize,
    // INVARIANT: forall i . data[i] < palette.len()
    // INVARIANT: data.len() = width * height, row-major.
    data: Vec<usize>,
}

pub struct GBAImageView<'a> {
    owner: &'a GBAImage,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl GBAImage {
    pub fn validate(&self) -> Result<(), Error> {
        if self.data.len() != self.width * self.height {
            return Err(Error::DimensionMismatch);
        }

        if self.width % 8 != 0 && self.height % 8 != 0 {
            return Err(Error::BadDimensions);
        }

        if let Some(_idx) = self.data.iter().find(|&&idx| idx > 0xF) {
            return Err(Error::BadColorIndex);
        }

        Ok(())
    }

    pub fn pixel_at(&self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }

        self.data.get(y * self.width + x).copied()
    }

    pub fn color_at(&self, x: usize, y: usize) -> Option<Color> {
        self.pixel_at(x, y).and_then(|idx| self.palette.lookup(idx))
    }

    pub fn tiles<'a>(&'a self) -> impl Iterator<Item = GBAImageView<'a>> {
        Tiles {
            owner: self,
            x: 0,
            y: 0,
        }
    }

    pub fn pixels<'a>(&'a self) -> impl Iterator<Item = usize> + 'a {
        PixelIterator {
            owner: self,
            x_offs: 0,
            y_offs: 0,
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }

    pub fn view(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> GBAImageView {
        GBAImageView {
            owner: self,
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_generic_image<V>(
        img: &V,
        colors: Option<Palette>,
    ) -> Result<Self, Error>
    where
        V: GenericImageView,
        V::Pixel: Pixel<Subpixel = u8>,
    {
        let (fixed_palette, mut colors) = match colors {
            None => (false, HashMap::new()),
            Some(colors) => (
                true,
                colors
                    .0
                    .iter()
                    .take(16)
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
        Self::from_generic_image(img, None)
    }

    pub fn with_known_palette<V>(
        img: &V,
        palette: Palette,
    ) -> Result<Self, Error>
    where
        V: GenericImageView,
        V::Pixel: Pixel<Subpixel = u8>,
    {
        Self::from_generic_image(img, Some(palette))
    }
}

impl<'owner> GBAImageView<'owner> {
    pub fn pixel_at(&self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }

        let true_x = self.x + x;
        let true_y = self.y + y;
        self.owner.pixel_at(true_x, true_y)
    }

    pub fn color_at(&self, x: usize, y: usize) -> Option<Color> {
        self.pixel_at(x, y)
            .and_then(|idx| self.owner.palette.lookup(idx))
    }

    pub fn pixels<'a>(&'a self) -> impl Iterator<Item = usize> + 'owner {
        PixelIterator {
            owner: self.owner,
            x_offs: self.x,
            y_offs: self.y,
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }
}

struct Tiles<'a> {
    owner: &'a GBAImage,
    x: usize,
    y: usize,
}

struct PixelIterator<'a> {
    owner: &'a GBAImage,
    x_offs: usize,
    y_offs: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl<'owner> Iterator for Tiles<'owner> {
    type Item = GBAImageView<'owner>;

    fn next(&mut self) -> Option<Self::Item> {
        let true_x = self.x * 8;
        let true_y = self.y * 8;

        if true_y >= self.owner.height {
            return None;
        }

        self.x += 1;

        if self.x * 8 >= self.owner.width {
            self.x = 0;
            self.y += 1;
        }

        Some(self.owner.view(true_x, true_y, 8, 8))
    }
}

impl<'owner> Iterator for PixelIterator<'owner> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let true_x = self.x + self.x_offs;
        let true_y = self.y + self.y_offs;

        if self.y >= self.height {
            return None;
        }

        self.x += 1;

        if self.x >= self.width {
            self.x = 0;
            self.y += 1;
        }

        self.owner.pixel_at(true_x, true_y)
    }
}

pub fn parse_palette_string(s: impl AsRef<str>) -> Option<Palette> {
    let s = s.as_ref();
    if s.len() != 64 {
        return None;
    }

    // CR-someday cam: It wouldn't be too hard to do this check while parsing
    // the colors but it _really_ doesn't matter.
    if !s.chars().any(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    s.chars()
        .tuples::<(_, _, _, _)>()
        .map(|(a, b, c, d)| {
            u16::from_str_radix(format!("{}{}{}{}", c, d, a, b).as_str(), 16)
                .map(Color::from_16bit)
                .ok()
        })
        .collect::<Option<Vec<_>>>()
        .map(Palette::from)
}

pub fn read_colors_from_image<V>(img: &V) -> Palette
where
    V: GenericImageView,
    V::Pixel: Pixel<Subpixel = u8>,
{
    img.pixels()
        .take(16)
        .map(|(_x, _y, pxl)| Color::from(pxl))
        .collect()
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

    GBAImage::from_generic_image(&img, palette)
}

// TODO: do this as an iterator
pub fn encode_tiles<'img>(
    tiles: impl Iterator<Item = GBAImageView<'img>>,
) -> Vec<u8> {
    tiles
        .flat_map(|tile| tile.pixels())
        .tuples::<(_, _)>()
        .map(|(a, b)| ((a & 0xF) | ((b & 0xF) << 4)) as u8)
        .collect()
}
