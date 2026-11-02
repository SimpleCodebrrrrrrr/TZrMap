//! Типы грунта и генерация карты.


/// Тип грунта тайла карты.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terrain {
    /// Вода
    Water,
    /// Песок
    Sand,
    /// Горы
    Mountain,
    /// Снег
    Snow,
    /// Поля
    Field,
    /// Лес
    Forest,
}

impl Terrain {
    pub const ALL: [Terrain; 6] = [
        Terrain::Water,
        Terrain::Sand,
        Terrain::Mountain,
        Terrain::Snow,
        Terrain::Field,
        Terrain::Forest,
    ];

    /// Цвет для отрисовки на мини-карте (RGB).
    pub fn color(&self) -> (u8, u8, u8) {
        match self {
            Terrain::Water => (30, 100, 200),    // синий
            Terrain::Sand => (235, 160, 60),     // оранжевый
            Terrain::Mountain => (120, 80, 50),  // коричневый
            Terrain::Snow => (245, 245, 250),    // белый
            Terrain::Field => (110, 190, 80),    // зелёный (поля)
            Terrain::Forest => (25, 95, 40),     // тёмно-зелёный (лес)
        }
    }

    /// Имя png-тайла локации: water.png, sand.png, ...
    pub fn tile_name(&self) -> &'static str {
        match self {
            Terrain::Water => "water",
            Terrain::Sand => "sand",
            Terrain::Mountain => "mountain",
            Terrain::Snow => "snow",
            Terrain::Field => "field",
            Terrain::Forest => "forest",
        }
    }

    /// Русское название.
    pub fn title(&self) -> &'static str {
        match self {
            Terrain::Water => "Вода",
            Terrain::Sand => "Песок",
            Terrain::Mountain => "Горы",
            Terrain::Snow => "Снег",
            Terrain::Field => "Поля",
            Terrain::Forest => "Лес",
        }
    }
}

/// Координата тайла (долгота/широта в дискретных единицах, -180..=180).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TilePos {
    pub lon: i32,
    pub lat: i32,
}

/// Карта: координаты от -180 до 180 включительно (361x361 пикселей/тайлов).
/// По умолчанию — песок, с водой по краям (полоса шириной `WATER_BORDER`).
pub struct WorldMap {
    pub size: i32, // = 361
    data: Vec<Terrain>,
}

/// Ширина водяной границы по краям карты.
pub const WATER_BORDER: i32 = 12;

impl WorldMap {
    pub fn new() -> Self {
        let size = 361; // -180..=180
        let mut data = vec![Terrain::Sand; (size * size) as usize];
        for lat in 0..size {
            for lon in 0..size {
                let edge = lon.min(lat).min(size - 1 - lon).min(size - 1 - lat);
                if edge < WATER_BORDER {
                    data[(lat * size + lon) as usize] = Terrain::Water;
                }
            }
        }
        Self { size, data }
    }

    /// Индекс по глобальной координате (-180..=180).
    fn idx(&self, lon: i32, lat: i32) -> usize {
        ((lat + 180) * self.size + (lon + 180)) as usize
    }

    pub fn get(&self, lon: i32, lat: i32) -> Terrain {
        self.data[self.idx(lon, lat)]
    }

    pub fn set(&mut self, lon: i32, lat: i32, t: Terrain) {
        let i = self.idx(lon, lat);
        self.data[i] = t;
    }
}
