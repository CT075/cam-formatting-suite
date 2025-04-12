use anyhow::{anyhow, bail, Result};
use tiled::{self, FiniteTileLayer, PropertyValue, TileLayer};

use crate::femap::*;

enum MainLayerState<'a> {
    NotFound,
    Candidate(String, FiniteTileLayer<'a>, tiled::Properties),
    Found(Vec<Tile>),
}

pub fn process_femap<'a>(map: &'a tiled::Map) -> Result<Map> {
    let mut width = Err(anyhow!("unable to determine map width"));
    let mut height = Err(anyhow!("unable to determine map height"));
    let mut main_layer = MainLayerState::NotFound;
    let mut map_changes: Vec<(String, MapChange)> = Vec::new();
    let mut properties: Properties = Default::default();

    for layer in map.layers() {
        // TODO: if there is a main layer that isn't suitable, say why

        let ftlayer = match layer.as_tile_layer() {
            Some(TileLayer::Finite(ftlayer)) => ftlayer,
            _ => continue,
        };

        if layer.name.to_lowercase() == "main"
            || layer
                .properties
                .iter()
                .any(|(s, _)| s.to_lowercase() == "main")
        {
            if let MainLayerState::Found(_) = main_layer {
                bail!("map has multiple main layers!");
            }

            main_layer = MainLayerState::Found(make_main_tile_layer(&ftlayer)?);

            // CR-someday cam: Populate from tileset name
            properties.populate_from(layer.properties.iter().filter_map(
                |(k, v)| render_property_value(v).map(|s| (k.to_string(), s)),
            ))?;
            width = Ok(ftlayer.width() as usize);
            height = Ok(ftlayer.height() as usize);
            continue;
        }
        match main_layer {
            MainLayerState::NotFound => {
                main_layer = MainLayerState::Candidate(
                    layer.name.clone(),
                    ftlayer,
                    layer.properties.clone(),
                );
                continue;
            }
            MainLayerState::Found(_) => (),
            MainLayerState::Candidate(ref name, other, _) => {
                map_changes.push((name.clone(), make_map_change(&other)?))
            }
        }
        map_changes.push((layer.name.clone(), make_map_change(&ftlayer)?))
    }

    let tiles = match main_layer {
        MainLayerState::NotFound => bail!("no main layer!"),
        MainLayerState::Candidate(_, ftlayer, layer_properties) => {
            properties.populate_from(layer_properties.iter().filter_map(
                |(k, v)| render_property_value(v).map(|s| (k.to_string(), s)),
            ))?;
            width = Ok(ftlayer.width() as usize);
            height = Ok(ftlayer.height() as usize);
            make_main_tile_layer(&ftlayer)?
        }
        MainLayerState::Found(data) => data,
    };

    Map::new(width?, height?, map_changes, tiles, properties)
}

fn gid_to_tile(gid: Option<u32>) -> Result<Tile> {
    match gid {
        None => return Ok(Tile::new(0)),
        Some(gid) => Ok(Tile::new(u16::try_from(gid * 4)?)),
    }
}

fn make_main_tile_layer<'a>(layer: &FiniteTileLayer<'a>) -> Result<Vec<Tile>> {
    TileIterator::all(layer).to_tiles()
}

fn make_map_change<'a>(layer: &FiniteTileLayer<'a>) -> Result<MapChange> {
    let (mut xmin, mut xmax, mut ymin, mut ymax) =
        (layer.width() as usize, 0, layer.height() as usize, 0);

    for (x, y, gid) in TileIterator::all(layer) {
        if let Some(_) = gid {
            xmin = usize::min(x, xmin);
            xmax = usize::max(x, xmax);
            ymin = usize::min(y, ymin);
            ymax = usize::max(y, ymax);
        }
    }

    let width = xmax - xmin + 1;
    let height = ymax - ymin + 1;

    let params = WindowParams {
        width,
        height,
        base_x: xmin,
        base_y: ymin,
    };

    let tiles: Vec<_> = TileIterator::window(layer, params).to_tiles()?;

    Ok(MapChange::new(xmin, ymin, width, height, tiles)?)
}

struct TileIterator<'a, 'map> {
    x: usize,
    y: usize,
    base_x: usize,
    base_y: usize,
    width: usize,
    height: usize,
    src: &'a FiniteTileLayer<'map>,
}

struct WindowParams {
    base_x: usize,
    base_y: usize,
    width: usize,
    height: usize,
}

impl<'a, 'map> TileIterator<'a, 'map> {
    fn all(src: &'a FiniteTileLayer<'map>) -> Self {
        Self {
            x: 0,
            y: 0,
            src,
            base_x: 0,
            base_y: 0,
            width: src.width() as usize,
            height: src.height() as usize,
        }
    }

    fn window(src: &'a FiniteTileLayer<'map>, params: WindowParams) -> Self {
        let WindowParams {
            base_x,
            base_y,
            width,
            height,
        } = params;

        Self {
            src,
            x: 0,
            y: 0,
            base_x,
            base_y,
            width,
            height,
        }
    }

    fn to_tiles<I>(self) -> Result<I>
    where
        I: FromIterator<Tile>,
    {
        self.map(|(_x, _y, gid)| gid_to_tile(gid))
            .collect::<Result<I>>()
    }
}

impl<'a, 'map> Iterator for TileIterator<'a, 'map> {
    type Item = (usize, usize, Option<u32>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.x >= self.width {
            self.x = 0;
            self.y += 1;
        }

        if self.y >= self.height {
            return None;
        }

        let x = self.x + self.base_x;
        let y = self.y + self.base_y;

        let result =
            (x, y, self.src.get_tile(x as i32, y as i32).map(|t| t.id()));

        self.x += 1;

        Some(result)
    }
}

fn render_property_value(p: &PropertyValue) -> Option<String> {
    match p {
        PropertyValue::BoolValue(t) => Some(format!("{}", t)),
        PropertyValue::FloatValue(t) => Some(format!("{}", t)),
        PropertyValue::IntValue(t) => Some(format!("{}", t)),
        PropertyValue::StringValue(t) => Some(format!("{}", t)),
        PropertyValue::ObjectValue(t) => Some(format!("{}", t)),
        PropertyValue::FileValue(_) => None,
        PropertyValue::ColorValue(_) => None,
        PropertyValue::ClassValue { .. } => None,
    }
}
