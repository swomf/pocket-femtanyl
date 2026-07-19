use std::{collections::HashMap, io::Cursor};

use anyhow::{Context, Result};
use quick_xml::{Reader, events::Event};

const SPRITESHEET_PNG: &[u8] = include_bytes!("../assets/femt.png");
const SPRITESHEET_XML: &str = include_str!("../assets/femt.xml");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animation {
    Idle,
    Left,
    Down,
    Up,
    Right,
}

impl Animation {
    pub const ALL: [Self; 5] = [Self::Idle, Self::Left, Self::Down, Self::Up, Self::Right];

    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Left => "left",
            Self::Down => "down",
            Self::Up => "up",
            Self::Right => "right",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AnimationFrame {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct Spritesheet {
    pub width: u32,
    pub rgba: Vec<u8>, // identify -verbose femt.png | grep Depth
    animations: HashMap<String, Vec<AnimationFrame>>,
}

impl Spritesheet {
    pub fn embedded() -> Self {
        // this area shouldnt ever fail
        let decoder = png::Decoder::new(Cursor::new(SPRITESHEET_PNG));
        let mut reader = decoder.read_info().expect("doesnt look like a png");
        let mut rgba = vec![0; reader.output_buffer_size().expect("png too big")];
        let info = reader.next_frame(&mut rgba).expect("couldnt decode PNG");
        rgba.truncate(info.buffer_size());

        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            // uncompressed 8bit rgba is the best setup for memory here
            // identify -verbose femt.png | grep Depth
            panic!("i want an 8bit rgba png, why are you fucking around?");
        }

        Self {
            width: info.width,
            rgba,
            animations: parse_frames(SPRITESHEET_XML),
        }
    }

    pub fn frames(&self, animation: Animation) -> &[AnimationFrame] {
        self.animations
            .get(animation.name())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

fn parse_frames(xml: &str) -> HashMap<String, Vec<AnimationFrame>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut animations = HashMap::<String, Vec<AnimationFrame>>::new();

    loop {
        match reader.read_event().expect("invalid spritesheet XML") {
            Event::Empty(element) if element.name().as_ref() == b"SubTexture" => {
                let mut values = HashMap::new();
                for attribute in element.attributes() {
                    let attribute = attribute.expect("invalid spritesheet attribute");
                    values.insert(
                        String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                        attribute
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Explicit1_0,
                                reader.decoder(),
                            )
                            .unwrap()
                            .into_owned(),
                    );
                }

                let name = values.get("name").unwrap();
                let animation = name.trim_end_matches(|character: char| character.is_ascii_digit());
                let number = |key: &str| -> Result<u32> {
                    values
                        .get(key) // not that bad but shouldnt happen
                        .with_context(|| format!("frame is missing {key}"))?
                        .parse()
                        .with_context(|| format!("frame has invalid {key}"))
                };
                animations
                    .entry(animation.to_owned())
                    .or_default()
                    .push(AnimationFrame {
                        x: number("x").unwrap(),
                        y: number("y").unwrap(),
                        width: number("width").unwrap(),
                        height: number("height").unwrap(),
                    });
            }
            Event::Eof => break,
            _ => {}
        }
    }
    animations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_expected_animation_lengths() {
        // seee femt.xml
        let frames = parse_frames(SPRITESHEET_XML);
        assert_eq!(frames["idle"].len(), 9);
        assert_eq!(frames["left"].len(), 7);
        assert_eq!(frames["down"].len(), 7);
        assert_eq!(frames["up"].len(), 8);
        assert_eq!(frames["right"].len(), 7);
    }
}
