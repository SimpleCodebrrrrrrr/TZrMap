//! Генерация png-тайлов для карты (если пользователь не положил свои в img/).

use image::{ImageBuffer, Rgb, RgbImage, Rgba, RgbaImage};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::path::Path;

use crate::terrain::Terrain;

const TILE: u32 = 64;

fn noise(img: &mut RgbImage, base: (u8, u8, u8), amp: u8, seed: u64) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    for px in img.pixels_mut() {
        let d: f32 = rng.gen();
        let d = d.mul_add(2.0, -1.0) * amp as f32;
        let c = |v: u8| (v as i32 + d as i32).clamp(0, 255) as u8;
        *px = Rgb([c(base.0), c(base.1), c(base.2)]);
    }
}

fn water(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([30u8, 100, 200]));
    noise(&mut img, (30, 100, 200), 18, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xABCD);
    // светлые «волны»
    for _ in 0..5 {
        let y = rng.gen_range(0..TILE) as usize;
        for x in 0..TILE as usize {
            let off = ((x as f32 / 8.0).sin() * 2.0) as i32;
            let yy = (y as i32 + off).rem_euclid(TILE as i32) as usize;
            let p = img.get_pixel(x as u32, yy as u32);
            img.put_pixel(
                x as u32,
                yy as u32,
                Rgb([p.0[0].saturating_add(35), p.0[1].saturating_add(35), p.0[2].min(255)]),
            );
        }
    }
    img
}

fn sand(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([235u8, 160, 60]));
    noise(&mut img, (235, 160, 60), 14, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x1234);
    // редкие травинки
    for _ in 0..6 {
        let x = rng.gen_range(2..TILE - 2) as usize;
        let y = rng.gen_range(2..TILE - 2) as usize;
        img.put_pixel(x as u32, y as u32, Rgb([70, 130, 50]));
    }
    img
}

fn mountain(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([120u8, 80, 50]));
    noise(&mut img, (120, 80, 50), 16, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x55AA);
    // силуэты скал
    for _ in 0..4 {
        let cx = rng.gen_range(8..TILE - 8) as i32;
        let h: i32 = rng.gen_range(12..28);
        for dx in -h..=h {
            let dy = h - dx.abs();
            let x = cx + dx;
            let y = (TILE as i32 / 2) - dy / 2;
            if x >= 0 && x < TILE as i32 {
                for yy in y..(y + dy.max(1)) {
                    if yy >= 0 && yy < TILE as i32 {
                        img.put_pixel(x as u32, yy as u32, Rgb([90, 60, 40]));
                    }
                }
            }
        }
    }
    img
}

fn snow(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([245u8, 245, 250]));
    noise(&mut img, (245, 245, 250), 6, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0x777);
    // следы
    for _ in 0..5 {
        let x = rng.gen_range(4..TILE - 4) as usize;
        let y = rng.gen_range(4..TILE - 4) as usize;
        img.put_pixel(x as u32, y as u32, Rgb([210, 215, 225]));
    }
    img
}

fn field(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([110u8, 190, 80]));
    noise(&mut img, (110, 190, 80), 14, seed);
    // борозды
    for y in (4..TILE).step_by(8) {
        for x in 0..TILE {
            let p = img.get_pixel(x, y);
            img.put_pixel(
                x,
                y,
                Rgb([p.0[0].saturating_sub(25), p.0[1].saturating_sub(30), p.0[2].saturating_sub(20)]),
            );
        }
    }
    img
}

fn forest(seed: u64) -> RgbImage {
    let mut img = ImageBuffer::from_pixel(TILE, TILE, Rgb([25u8, 95, 40]));
    noise(&mut img, (25, 95, 40), 12, seed);
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xF0F0);
    // кроны деревьев
    for _ in 0..10 {
        let cx = rng.gen_range(4..TILE - 4) as i32;
        let cy = rng.gen_range(4..TILE - 4) as i32;
        let r = rng.gen_range(3..6);
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let x = cx + dx;
                    let y = cy + dy;
                    if x >= 0 && y >= 0 && x < TILE as i32 && y < TILE as i32 {
                        img.put_pixel(x as u32, y as u32, Rgb([45, 130, 60]));
                    }
                }
            }
        }
    }
    img
}

/// Создаёт каталог `dir` и кладёт в него базовые png-тайлы для каждого грунта,
/// а также заготовки строений с альфа-каналом (zavod/lab/tpk/dom) — только для
/// тех файлов, которых ещё нет.
pub fn ensure_tiles(dir: &Path) {
    std::fs::create_dir_all(dir).ok();
    let variants: [(Terrain, fn(u64) -> RgbImage); 6] = [
        (Terrain::Water, water),
        (Terrain::Sand, sand),
        (Terrain::Mountain, mountain),
        (Terrain::Snow, snow),
        (Terrain::Field, field),
        (Terrain::Forest, forest),
    ];
    for (t, gen) in variants {
        let path = dir.join(format!("{}.png", t.tile_name()));
        if !path.exists() {
            let img = gen(t as u64 * 977 + 13);
            if image::DynamicImage::ImageRgb8(img)
                .save_with_format(&path, image::ImageFormat::Png)
                .is_err()
            {
                eprintln!("Не удалось записать {}", path.display());
            }
        }
    }
    // заготовки строений (png с альфа-каналом, рисуются в четверти тайла)
    let buildings = [
        ("zavod", (150u8, 150, 160)),  // завод — серый корпус с трубой
        ("lab", (240, 240, 245)),      // лаборатория — белый купол
        ("tpk", (220, 40, 40)),        // ТПК — красная будка
        ("dom", (180, 120, 70)),       // дом — коричневый коттедж
    ];
    for (name, color) in buildings {
        let path = dir.join(format!("{}.png", name));
        if !path.exists() {
            let img = building_placeholder(name, color);
            if image::DynamicImage::ImageRgba8(img)
                .save_with_format(&path, image::ImageFormat::Png)
                .is_err()
            {
                eprintln!("Не удалось записать {}", path.display());
            }
        }
    }
}

/// Простая заготовка здания с прозрачным фоном (альфа-канал).
fn building_placeholder(kind: &str, color: (u8, u8, u8)) -> RgbaImage {
    let s = TILE;
    let mut img = ImageBuffer::from_pixel(s, s, Rgba([0, 0, 0, 0]));
    let dark = (
        color.0.saturating_sub(50),
        color.1.saturating_sub(50),
        color.2.saturating_sub(50),
    );
    let px = |img: &mut RgbaImage, x: i32, y: i32, c: (u8, u8, u8)| {
        if x >= 0 && y >= 0 && x < s as i32 && y < s as i32 {
            img.put_pixel(x as u32, y as u32, Rgba([c.0, c.1, c.2, 255]));
        }
    };
    match kind {
        "zavod" => {
            // корпус + две трубы
            for x in 8..s as i32 - 8 {
                for y in (s as i32 - 26)..(s as i32 - 8) {
                    px(&mut img, x, y, color);
                }
            }
            for cx in [14i32, 26, 38] {
                for y in 6..(s as i32 - 26) {
                    px(&mut img, cx, y, dark);
                    px(&mut img, cx + 1, y, dark);
                    px(&mut img, cx + 2, y, dark);
                }
            }
        }
        "lab" => {
            // белый купол на основании
            let cx = s as i32 / 2;
            let cy = s as i32 - 14;
            let r = 16i32;
            for dy in -r..=0 {
                for dx in -r..=r {
                    if dx * dx + dy * dy <= r * r {
                        px(&mut img, cx + dx, cy + dy, color);
                    }
                }
            }
            for x in (cx - r - 2)..(cx + r + 3) {
                for y in cy..cy + 6 {
                    px(&mut img, x, y, dark);
                }
            }
        }
        "tpk" => {
            // будка с крышей
            for x in 14..(s as i32 - 14) {
                for y in 18..(s as i32 - 8) {
                    px(&mut img, x, y, color);
                }
            }
            for x in 10..(s as i32 - 10) {
                for y in 12..18 {
                    px(&mut img, x, y, dark);
                }
            }
            // дверь
            for x in ((s as i32 / 2) - 3)..((s as i32 / 2) + 3) {
                for y in (s as i32 - 20)..(s as i32 - 8) {
                    px(&mut img, x, y, (40, 40, 40));
                }
            }
        }
        _ => {
            // дом: стены + треугольная крыша
            for x in 12..(s as i32 - 12) {
                for y in 28..(s as i32 - 8) {
                    px(&mut img, x, y, color);
                }
            }
            let apex_y = 10i32;
            for y in apex_y..28 {
                let half = ((y - apex_y) as f32 / (28 - apex_y) as f32 * 18.0) as i32;
                for x in (s as i32 / 2 - half)..(s as i32 / 2 + half) {
                    px(&mut img, x, y, dark);
                }
            }
        }
    }
    img
}
