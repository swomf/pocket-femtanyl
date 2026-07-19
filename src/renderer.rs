use std::{collections::HashMap, time::Duration};

use anyhow::{Context as _, Result}; // context as _ to not clash with Renderer
use cairo::{Context, Format, ImageSurface, Operator};
use noise::{NoiseFn, Perlin};

use crate::spritesheet::{Animation, AnimationFrame, Spritesheet};

const FRAME_RATE: f64 = 24.0; // characters/femt.json
const SHADOW_OFFSET: i32 = 15; // stages/bg.lua
const SHADOW_ALPHA: u16 = 191; // stages/bg.lua (0.75 * 255); see prepare_surface
const CHROMATIC_OFFSET: i32 = 4; // shaders/chromaticAbberation.frag

// shadow and chromatic stuff are precomputed
// rest is framewise

pub struct Renderer {
    spritesheet: Spritesheet,
    surfaces: HashMap<AnimationFrame, ImageSurface>,
    noise_x: Perlin,
    noise_y: Perlin,
    started_at: std::time::Instant,
}

impl Renderer {
    pub fn new(mut spritesheet: Spritesheet) -> Result<Self> {
        let mut surfaces = HashMap::new();
        for animation in Animation::ALL {
            for frame in spritesheet.frames(animation) {
                if !surfaces.contains_key(frame) {
                    surfaces.insert(*frame, prepare_surface(&spritesheet, frame)?);
                }
            }
        }

        // free rgba png (per frame surfaces hold our pixels already)
        spritesheet.rgba = Vec::new();

        Ok(Self {
            spritesheet,
            surfaces,
            noise_x: Perlin::new(1338),
            noise_y: Perlin::new(1337),
            started_at: std::time::Instant::now(),
        })
    }

    pub fn render(
        &mut self,
        context: &Context,
        width: i32,
        height: i32,
        animation: Animation,
        elapsed: Duration,
    ) -> Result<()> {
        context.save()?;
        context.set_operator(Operator::Clear);
        context.paint()?;
        context.restore()?;

        let frames = self.spritesheet.frames(animation);
        let frame = frames
            .get((elapsed.as_secs_f64() * FRAME_RATE).floor() as usize % frames.len())
            .unwrap();
        let surface = self
            .surfaces
            .get(frame)
            .context("animation frame was not prepared")?;
        let scale = (width as f64 / frame.width as f64).min(height as f64 / frame.height as f64);
        let time = self.started_at.elapsed().as_secs_f64();
        let perlin_y = 2.1 + time * 0.36;
        // from perlinCamera.lua wigglesintensity=15
        let camera_x = self.noise_x.get([1.1, perlin_y, 3.1]) * 15.0;
        let camera_y = self.noise_y.get([1.1, perlin_y, 3.1]) * 15.0;

        context.save()?;
        context.set_operator(Operator::Over);
        context.translate(
            width as f64 * 0.5 + camera_x,
            height as f64 * 0.5 + camera_y,
        );
        context.scale(scale, scale);
        context.set_source_surface(
            surface,
            -(frame.width as f64) * 0.5,
            -(frame.height as f64) * 0.5,
        )?;
        context.paint()?;
        context.restore()?;
        Ok(())
    }
}

// precompute shadow and chromatic aberration then store as surface
fn prepare_surface(spritesheet: &Spritesheet, frame: &AnimationFrame) -> Result<ImageSurface> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let mut shadowed = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let color = spritesheet_pixel(spritesheet, frame, x as i32, y as i32);
            let shadow = spritesheet_pixel(
                spritesheet,
                frame,
                x as i32 - SHADOW_OFFSET, // lesser is bottom right
                y as i32 - SHADOW_OFFSET,
            );
            let shadow_alpha = shadow[3] as u16 * SHADOW_ALPHA / 255;
            let alpha = color[3] as u16 + shadow_alpha * (255 - color[3] as u16) / 255;
            //let alpha = (1 - shadow_alpha) + shadow_alpha * (255 - color[3] as u16) / 255;
            let index = (y * width + x) * 4;
            shadowed[index..index + 4].copy_from_slice(&[
                color[0],
                color[1],
                color[2],
                alpha as u8,
            ]);
        }
    }

    // num of bytes you skip in memory to move from a row of pixels to the next one
    // (i.e. width of scanline in bytes, including padding)
    let stride = Format::ARgb32.stride_for_width(frame.width)? as usize;
    let mut pixels = vec![0u8; stride * height];
    for y in 0..height {
        for x in 0..width {
            let center = local_pixel(&shadowed, width, height, x as i32, y as i32);
            let positive = local_pixel(
                &shadowed,
                width,
                height,
                x as i32 + CHROMATIC_OFFSET,
                y as i32,
            );
            let negative = local_pixel(
                &shadowed,
                width,
                height,
                x as i32 - CHROMATIC_OFFSET,
                y as i32,
            );
            let alpha = center[3] as u32;
            let red = positive[0] as u32 * alpha / 255;
            let green = center[1] as u32 * alpha / 255;
            let blue = negative[2] as u32 * alpha / 255;
            let argb = (alpha << 24) | (red << 16) | (green << 8) | blue;
            let index = y * stride + x * 4;
            pixels[index..index + 4].copy_from_slice(&argb.to_ne_bytes());
        }
    }

    Ok(ImageSurface::create_for_data(
        pixels,
        Format::ARgb32,
        frame.width as i32,
        frame.height as i32,
        stride as i32,
    )?)
}

fn spritesheet_pixel(spritesheet: &Spritesheet, frame: &AnimationFrame, x: i32, y: i32) -> [u8; 4] {
    if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
        return [0; 4];
    }
    let spritesheet_x = frame.x as usize + x as usize;
    let spritesheet_y = frame.y as usize + y as usize;
    let index = (spritesheet_y * spritesheet.width as usize + spritesheet_x) * 4;
    spritesheet.rgba[index..index + 4].try_into().unwrap()
}

fn local_pixel(pixels: &[u8], width: usize, height: usize, x: i32, y: i32) -> [u8; 4] {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return [0; 4];
    }
    let index = (y as usize * width + x as usize) * 4;
    pixels[index..index + 4].try_into().unwrap()
}
