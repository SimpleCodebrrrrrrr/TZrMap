//! TZrMap — GUI редактора карты TimeZero:Reborn.
//!
//! Структура окна:
//!  * Основной экран разделён на две половины (GtkPaned), левая — фиксированной ширины.
//!  * Левая часть:
//!      - сверху: квадратная мини-карта координат −180…180, каждый пиксель =
//!        цвет грунта (синий — вода, оранжевый — песок, коричневый — горы,
//!        белый — снег, зелёный — поля, тёмно-зелёный — лес);
//!      - снизу: tabControl (GtkNotebook) с двумя вкладками:
//!          1) текстовое поле комментария;
//!          2) форма локации: 10 чекбоксов (виды монстров), spin-input уровня
//!             радиации и уровня монстров, 3 radiobutton (вид дорог),
//!             6 чекбоксов (характеристики локации), combobox вида значка.
//!  * Правая часть (растягивается): карта — тайловое поле, каждая клетка
//!    отображается png-изображением из каталога `img/` рядом с исполняемым
//!    файлом (или заливкой по типу грунта).
//!  * Характеристики локации «завод», «лаборатория», «ТПК», «дом» загружают
//!    соответствующие png (с альфа-каналом) из того же каталога и размещают
//!    их в углу тайла, занимая четверть его площади.

mod terrain;
mod tiles_gen;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::cairo;
use libadwaita::{self as adw, prelude::*};

use terrain::{Terrain, TilePos, WorldMap};

/// Размер клетки основной карты в пикселях.
const TILE_PX: i32 = 48;
/// Ширина мини-карты в пикселях (1 пиксель = 1 тайл, всего 361x361).
const MINIMAP_SIZE: i32 = 361;
/// Ширина левой панели по умолчанию.
const LEFT_WIDTH: i32 = 420;

// ---------------------------------------------------------------------------
// Кэш png-изображений, декодированных в cairo ImageSurface (ARGB32)
// ---------------------------------------------------------------------------

/// Ожидается `<dir>/<lon>_<lat>.png`, либо общий тайл `<dir>/<тип>.png`.
fn tile_png_path(dir: &Path, pos: TilePos, terrain: Terrain) -> Option<PathBuf> {
    let per_tile = dir.join(format!("{}_{}.png", pos.lon, pos.lat));
    if per_tile.exists() {
        return Some(per_tile);
    }
    let common = dir.join(format!("{}.png", terrain.tile_name()));
    if common.exists() {
        return Some(common);
    }
    None
}

#[derive(Default)]
struct SurfaceCache {
    surfaces: HashMap<String, Option<Rc<cairo::ImageSurface>>>,
}

impl SurfaceCache {
    /// Поверхность для файла `path` (кэшируется по имени файла).
    fn get(&mut self, dir: &Path, name: &str) -> Option<Rc<cairo::ImageSurface>> {
        if !self.surfaces.contains_key(name) {
            let loaded = load_png_surface(&dir.join(name)).map(Rc::new);
            self.surfaces.insert(name.to_string(), loaded);
        }
        self.surfaces.get(name)?.clone()
    }
}

/// Кэш png-тайлов грунтов (по типу) и png строений локаций (по имени файла).
struct PngTileCache {
    dir: PathBuf,
    terrain_tiles: SurfaceCache,
    buildings: RefCell<SurfaceCache>,
}

impl PngTileCache {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            terrain_tiles: SurfaceCache::default(),
            buildings: RefCell::new(SurfaceCache::default()),
        }
    }

    /// Возвращает cairo-поверхность с содержимым png-файла тайла (или None).
    fn surface(&mut self, pos: TilePos, terrain: Terrain) -> Option<Rc<cairo::ImageSurface>> {
        match tile_png_path(&self.dir, pos, terrain) {
            Some(p) => {
                let name = p.file_name()?.to_string_lossy().into_owned();
                self.terrain_tiles.get(&self.dir.clone(), &name)
            }
            None => None,
        }
    }

    /// Поверхность png-файла строения (`<dir>/<file>.png`), если он существует.
    fn building(&self, file: &str) -> Option<Rc<cairo::ImageSurface>> {
        self.buildings.borrow_mut().get(&self.dir, file)
    }
}

/// Декодирует png-файл и копирует пиксели в cairo ImageSurface (ARGB32).
fn load_png_surface(path: &Path) -> Option<cairo::ImageSurface> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let stride = cairo::Format::ARgb32.stride_for_width(w).ok()?;
    let len = (stride * h as i32) as usize;
    let mut buf = vec![0u8; len];
    for y in 0..h as usize {
        let row = &mut buf[y * stride as usize..][..w as usize * 4];
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            let p = img.get_pixel(x as u32, y as u32).0;
            let a = p[3] as u16;
            // straight RGBA -> premultiplied BGRA (формат ARGB32 в cairo — little-endian BGRA)
            px[0] = (p[2] as u16 * a / 255) as u8;
            px[1] = (p[1] as u16 * a / 255) as u8;
            px[2] = (p[0] as u16 * a / 255) as u8;
            px[3] = p[3];
        }
    }
    let surf =
        cairo::ImageSurface::create_for_data(buf, cairo::Format::ARgb32, w as i32, h as i32, stride)
            .ok()?;
    Some(surf)
}

// ---------------------------------------------------------------------------
// Данные выбранной локации
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LocData {
    comment: String,
    monsters: Vec<String>,
    radiation: i32,
    monster_level: i32,
    road: usize,
    features: Vec<String>,
    icon: String,
}

impl Default for LocData {
    fn default() -> Self {
        Self {
            comment: String::new(),
            monsters: Vec::new(),
            radiation: 0,
            monster_level: 0,
            road: 0,
            features: Vec::new(),
            icon: ICONS[0].to_string(),
        }
    }
}

const MONSTERS: [&str; 10] = [
    "Зомби",
    "Роботы",
    "Мутанты",
    "Дикая природа",
    "НЛО",
    "Огр",
    "Скорпионы",
    "Пауки",
    "Пришельцы",
    "Твари из бункера",
];

const ROADS: [&str; 3] = ["Шоссе", "Грунтовая дорога", "Нет дорог"];

/// Характеристики локации. Для «завод/лаборатория/ТПК/дом» в каталоге img/
/// ищется png с альфа-каналом (`file`), который рисуется в четверти тайла.
struct Feature {
    title: &'static str,
    file: Option<&'static str>,
}

const FEATURES: [Feature; 6] = [
    Feature {
        title: "Завод",
        file: Some("zavod"),
    },
    Feature {
        title: "Лаборатория",
        file: Some("lab"),
    },
    Feature {
        title: "ТПК",
        file: Some("tpk"),
    },
    Feature {
        title: "Дом",
        file: Some("dom"),
    },
    Feature {
        title: "Аномалии",
        file: None,
    },
    Feature {
        title: "Подземелье",
        file: None,
    },
];

const ICONS: [&str; 9] = [
    "Нет",
    "Человеческая база",
    "Лагерь бандитов",
    "Убежище",
    "Аномалия",
    "Врата миров",
    "Развалины",
    "Караван",
    "Особняк",
];

// ---------------------------------------------------------------------------
// Общее состояние приложения
// ---------------------------------------------------------------------------

type Hook = Rc<RefCell<Option<Box<dyn Fn()>>>>;

struct AppState {
    map: RefCell<WorldMap>,
    tile_cache: RefCell<PngTileCache>,
    selected: Cell<Option<(i32, i32)>>,
    locations: RefCell<HashMap<(i32, i32), LocData>>,
    minimap_area: gtk::DrawingArea,
    main_area: gtk::DrawingArea,
    info_label: gtk::Label,
    /// вызывается при смене выбранной клетки — обновляет обе вкладки
    load_hook: Hook,
}

impl AppState {
    fn areas(&self) -> (gtk::DrawingArea, gtk::DrawingArea) {
        (self.minimap_area.clone(), self.main_area.clone())
    }

    fn coords_text(&self) -> String {
        match self.selected.get() {
            Some((lon, lat)) => {
                let t = self.map.borrow().get(lon, lat);
                format!(
                    "Локация x={lon}, y={lat} · грунт: {} · ЛКМ по карте — выбрать клетку, перетаскивание — рисовать выбранным грунтом",
                    t.title()
                )
            }
            None => "Кликните по карте для выбора локации".to_string(),
        }
    }

    fn refresh(&self) {
        self.info_label.set_text(&self.coords_text());
        let (minimap, main) = self.areas();
        minimap.queue_draw();
        main.queue_draw();
    }

    fn select(&self, lon: i32, lat: i32) {
        if self.selected.get() != Some((lon, lat)) {
            self.selected.set(Some((lon, lat)));
            if let Some(f) = self.load_hook.borrow().as_ref() {
                f();
            }
        }
        self.refresh();
    }
}

fn run_hook(h: &Hook) {
    if let Some(f) = h.borrow().as_ref() {
        f();
    }
}

/// CheckButton с текстовой меткой (в GTK4 нет with_label — собираем вручную).
fn check_button(label: &str) -> gtk::CheckButton {
    let c = gtk::CheckButton::new();
    c.set_label(Some(label));
    c
}

/// ToggleButton с текстовой меткой.
fn toggle_button(label: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&gtk::Label::new(Some(label))));
    b
}

/// «Radiobutton» в GTK4: CheckButton, сгруппированный через group() для
/// взаимной исключительности (класс RadioButton удалён из GTK4).
fn radio_button(group: Option<&gtk::CheckButton>, label: &str) -> gtk::CheckButton {
    let c = gtk::CheckButton::builder().label(label).build();
    if let Some(g) = group {
        c.set_group(Some(g));
    }
    c
}

// ---------------------------------------------------------------------------
// Мини-карта (левая верхняя половина)
// ---------------------------------------------------------------------------

fn setup_minimap(app: &Rc<AppState>) -> gtk::Widget {
    let area = &app.minimap_area;
    area.set_content_width(MINIMAP_SIZE);
    area.set_content_height(MINIMAP_SIZE);
    area.set_hexpand(false);
    area.set_vexpand(false);

    {
        let app2 = app.clone();
        area.set_draw_func(move |_, cr: &cairo::Context, w, _h| {
            let map = app2.map.borrow();
            let scale = (w as f64) / 361.0;
            // фон (на случай щелей между пикселями)
            cr.set_source_rgb(0.5, 0.5, 0.5);
            let _ = cr.paint();
            for lat in -180..=180i32 {
                for lon in -180..=180i32 {
                    let (r, g, b) = map.get(lon, lat).color();
                    cr.set_source_rgb(r as f64 / 255., g as f64 / 255., b as f64 / 255.);
                    cr.rectangle(
                        (lon + 180) as f64 * scale,
                        (180 - lat) as f64 * scale,
                        scale.ceil() + 0.5,
                        scale.ceil() + 0.5,
                    );
                    let _ = cr.fill();
                }
            }
            // рамка выбранной клетки
            if let Some((lon, lat)) = app2.selected.get() {
                cr.set_source_rgba(1.0, 0.1, 0.1, 1.0);
                cr.set_line_width(2.0);
                cr.rectangle(
                    (lon + 180) as f64 * scale - 1.0,
                    (180 - lat) as f64 * scale - 1.0,
                    scale + 2.0,
                    scale + 2.0,
                );
                let _ = cr.stroke();
            }
        });
    }

    // клик по мини-карте выбирает локацию
    let gesture = gtk::GestureDrag::new();
    {
        let app2 = app.clone();
        let area3 = area.clone();
        gesture.connect_drag_update(move |_, x, y| {
            let alloc = area3.width();
            if alloc <= 0 {
                return;
            }
            let scale = alloc as f64 / 361.0;
            let lon = ((x / scale) as i32 - 180).clamp(-180, 180);
            let lat = (180 - (y / scale) as i32).clamp(-180, 180);
            app2.select(lon, lat);
        });
    }
    area.add_controller(gesture);

    let frame = gtk::Frame::new(None);
    frame.set_child(Some(area));
    frame.set_label_widget(Some(&gtk::Label::new(Some("Мини-карта (−180 … 180)"))));
    frame.set_halign(gtk::Align::Center);
    frame.set_valign(gtk::Align::Start);
    frame.upcast()
}

// ---------------------------------------------------------------------------
// Основная карта (правая половина) — тайловое поле с png-картинками
// ---------------------------------------------------------------------------

fn setup_main_map(app: &Rc<AppState>) -> gtk::Widget {
    let area = &app.main_area;
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_focus(true);

    // Палитра грунтов для «рисования»
    let palette = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    palette.set_margin_start(6);
    palette.set_margin_top(6);
    let palette_sel: Rc<Cell<usize>> = Rc::new(Cell::new(1)); // по умолчанию песок
    let labels = ["Вода", "Песок", "Горы", "Снег", "Поля", "Лес"];
    let buttons: Vec<gtk::ToggleButton> = Terrain::ALL
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let b = toggle_button(labels[i]);
            b.set_tooltip_text(Some(&format!("Рисовать грунтом «{}»", labels[i])));
            palette.append(&b);
            b
        })
        .collect();
    buttons[1].set_active(true);
    for (i, b) in buttons.iter().enumerate() {
        let sel = palette_sel.clone();
        let all = buttons.clone();
        b.connect_toggled(move |btn| {
            if btn.is_active() {
                sel.set(i);
            } else if sel.get() == i {
                btn.set_active(true); // последний активный нельзя снять
            }
            for (j, o) in all.iter().enumerate() {
                if j != sel.get() && o.is_active() {
                    o.set_active(false);
                }
            }
        });
    }

    let app2 = app.clone();
    area.set_draw_func(move |_, cr: &cairo::Context, w, h| {
        let map = app2.map.borrow();
        let locations = app2.locations.borrow();
        let mut cache = app2.tile_cache.borrow_mut();
        let cols = (w / TILE_PX).clamp(1, 361);
        let rows = (h / TILE_PX).clamp(1, 361);
        for row in 0..rows {
            for col in 0..cols {
                let lon = -180 + col;
                let lat = 180 - row;
                let terrain = map.get(lon, lat);
                let x = (col * TILE_PX) as f64;
                let y = (row * TILE_PX) as f64;

                // png-картинка локации, если есть (каталог img/ рядом с запуском)
                let mut drawn = false;
                if let Some(surf) = cache.surface(TilePos { lon, lat }, terrain) {
                    let sw = surf.width();
                    let sh = surf.height();
                    if sw > 0 && sh > 0 {
                        let _ = cr.save();
                        cr.translate(x, y);
                        cr.scale(
                            TILE_PX as f64 / sw as f64,
                            TILE_PX as f64 / sh as f64,
                        );
                        cr.set_source_surface(&*surf, 0.0, 0.0).ok();
                        cr.paint().ok();
                        let _ = cr.restore();
                        drawn = true;
                    }
                }
                if !drawn {
                    let (r, g, b) = terrain.color();
                    cr.set_source_rgb(r as f64 / 255., g as f64 / 255., b as f64 / 255.);
                    cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
                    let _ = cr.fill();
                }

                // строения локации: png с альфа-каналом в четверти тайла
                if let Some(loc) = locations.get(&(lon, lat)) {
                    let quarter = (TILE_PX as f64) / 2.0;
                    let mut slot = 0usize; // угол тайла для следующего строения
                    for fname in &loc.features {
                        let file = match FEATURES.iter().find(|f| f.title == fname) {
                            Some(f) => f.file,
                            None => None,
                        };
                        let Some(base) = file else { continue };
                        let Some(surf) = cache.building(base) else {
                            continue;
                        };
                        let sw = surf.width().max(1);
                        let sh = surf.height().max(1);
                        // положение в углу тайла: 0 — левый верхний,
                        // 1 — правый верхний, 2 — левый нижний, 3 — правый нижний
                        let px = x + if slot % 2 == 0 { 0.0 } else { quarter };
                        let py = y + if slot < 2 { 0.0 } else { quarter };
                        let _ = cr.save();
                        cr.translate(px, py);
                        cr.scale(quarter / sw as f64, quarter / sh as f64);
                        cr.set_source_surface(&*surf, 0.0, 0.0).ok();
                        cr.paint().ok();
                        let _ = cr.restore();
                        slot = (slot + 1) % 4;
                    }
                }

                // тонкая сетка тайлового поля
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.15);
                cr.set_line_width(1.0);
                cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
                let _ = cr.stroke();
            }
        }
        // подсветка выбранной клетки
        if let Some((lon, lat)) = app2.selected.get() {
            let x = ((lon + 180) * TILE_PX) as f64;
            let y = ((180 - lat) * TILE_PX) as f64;
            cr.set_source_rgba(1.0, 0.1, 0.1, 0.95);
            cr.set_line_width(3.0);
            cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
            let _ = cr.stroke();
        }
    });

    // выбор клетки и рисование грунтом
    let gesture = gtk::GestureDrag::new();
    {
        let app3 = app.clone();
        gesture.connect_drag_begin({
            let app4 = app.clone();
            move |_, x, y| {
                let lon = (x as i32) / TILE_PX - 180;
                let lat = 180 - (y as i32) / TILE_PX;
                if (-180..=180).contains(&lon) && (-180..=180).contains(&lat) {
                    app4.select(lon, lat);
                }
            }
        });
        let pal = palette_sel.clone();
        gesture.connect_drag_update(move |_, x, y| {
            let lon = (x as i32) / TILE_PX - 180;
            let lat = 180 - (y as i32) / TILE_PX;
            if (-180..=180).contains(&lon) && (-180..=180).contains(&lat) {
                let t = Terrain::ALL[pal.get()];
                if app3.map.borrow().get(lon, lat) != t {
                    app3.map.borrow_mut().set(lon, lat, t);
                    app3.refresh();
                }
            }
        });
    }
    area.add_controller(gesture);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(area));
    scroller.set_hexpand(true);
    scroller.set_vexpand(true);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vbox.append(&palette);
    vbox.append(&scroller);
    vbox.upcast()
}

// ---------------------------------------------------------------------------
// Вкладка 1: комментарий
// ---------------------------------------------------------------------------

fn make_comment_tab(app: &Rc<AppState>) -> gtk::Widget {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);

    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_vexpand(true);
    scroll.set_child(Some(&view));

    // сохранение текста в данные локации
    let state = app.clone();
    let saving = Rc::new(Cell::new(false));
    view.buffer().connect_changed({
        let saving = saving.clone();
        move |buf| {
            if saving.get() {
                return;
            }
            let Some(key) = state.selected.get() else { return };
            let text = buf.text(&buf.bounds().0, &buf.bounds().1, true);
            state.locations.borrow_mut().entry(key).or_default().comment = text.to_string();
        }
    });

    // загрузка комментария при смене клетки
    let app2 = app.clone();
    let hook: Box<dyn Fn()> = Box::new(move || {
        let comment = match app2.selected.get() {
            Some(k) => app2
                .locations
                .borrow()
                .get(&k)
                .map(|d| d.comment.clone())
                .unwrap_or_default(),
            None => String::new(),
        };
        let flag = saving.clone();
        flag.set(true);
        view.buffer().set_text(&comment);
        flag.set(false);
    });
    set_hook(&app.load_hook, hook);

    scroll.upcast()
}

// ---------------------------------------------------------------------------
// Вкладка 2: форма локации
// ---------------------------------------------------------------------------

fn make_form_tab(app: &Rc<AppState>) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);

    // --- Блок: виды монстров (10 checkbox) ---
    let mon_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    mon_box.append(&section_title("Виды монстров"));
    let mon_grid = gtk::Grid::new();
    mon_grid.set_column_spacing(16);
    mon_grid.set_row_spacing(2);
    let mon_checks: Vec<gtk::CheckButton> = MONSTERS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let c = check_button(name);
            mon_grid.attach(&c, (i / 5) as i32, (i % 5) as i32, 1, 1);
            c
        })
        .collect();
    mon_box.append(&mon_grid);
    content.append(&mon_box);

    // --- Input: уровень радиации и уровень монстров ---
    let inputs = gtk::Grid::new();
    inputs.set_column_spacing(10);
    inputs.set_row_spacing(6);
    inputs.attach(&section_title_inline("Уровень радиации:"), 0, 0, 1, 1);
    let rad_spin = gtk::SpinButton::with_range(0.0, 100.0, 1.0);
    inputs.attach(&rad_spin, 1, 0, 1, 1);
    inputs.attach(&section_title_inline("Уровень монстров:"), 0, 1, 1, 1);
    let lvl_spin = gtk::SpinButton::with_range(1.0, 50.0, 1.0);
    inputs.attach(&lvl_spin, 1, 1, 1, 1);
    content.append(&inputs);

    // --- Блок: вид дорог (3 radiobutton) ---
    let road_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    road_box.append(&section_title("Вид дорог"));
    let road_group = radio_button(None, ROADS[0]);
    road_group.set_active(true);
    let road_btns = vec![
        road_group.clone(),
        radio_button(Some(&road_group), ROADS[1]),
        radio_button(Some(&road_group), ROADS[2]),
    ];
    for b in &road_btns {
        road_box.append(b);
    }
    content.append(&road_box);

    // --- Блок: характеристики локации (6 checkbox) ---
    let feat_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    feat_box.append(&section_title("Характеристики локации"));
    let feat_grid = gtk::Grid::new();
    feat_grid.set_column_spacing(16);
    feat_grid.set_row_spacing(2);
    let feat_checks: Vec<gtk::CheckButton> = FEATURES
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let c = check_button(f.title);
            feat_grid.attach(&c, (i / 3) as i32, (i % 3) as i32, 1, 1);
            c
        })
        .collect();
    feat_box.append(&feat_grid);
    content.append(&feat_box);

    // --- Combobox: вид значка ---
    let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    icon_box.append(&section_title_inline("Вид значка:"));
    let icon_model = gtk::StringList::new(&ICONS);
    let icon_combo = gtk::DropDown::builder().model(&icon_model).selected(0).build();
    icon_combo.set_hexpand(false);
    icon_box.append(&icon_combo);
    content.append(&icon_box);

    // === Сохранение любых изменений в данные выбранной локации ===
    let state = app.clone();
    let loading = Rc::new(Cell::new(false));
    let do_save = {
        let state = state.clone();
        let loading = loading.clone();
        let mon = mon_checks.clone();
        let feat = feat_checks.clone();
        let roads = road_btns.clone();
        let rad = rad_spin.clone();
        let lvl = lvl_spin.clone();
        let combo = icon_combo.clone();
        Rc::new(move || {
            if loading.get() {
                return;
            }
            let Some(key) = state.selected.get() else { return };
            let mut d = state.locations.borrow().get(&key).cloned().unwrap_or_default();
            d.monsters = MONSTERS
                .iter()
                .enumerate()
                .filter(|(i, _)| mon[*i].is_active())
                .map(|(_, m)| (*m).to_string())
                .collect();
            d.radiation = rad.value() as i32;
            d.monster_level = lvl.value() as i32;
            d.road = roads.iter().position(|b| b.is_active()).unwrap_or(0);
            d.features = FEATURES
                .iter()
                .enumerate()
                .filter(|(i, _)| feat[*i].is_active())
                .map(|(_, f)| f.title.to_string())
                .collect();
            if let Some(s) = combo.selected_item().and_downcast::<gtk::StringObject>() {
                d.icon = s.string().to_string();
            }
            state.locations.borrow_mut().insert(key, d);
        })
    };
    {
        let save = do_save.clone();
        for w in mon_checks
            .iter()
            .map(|x| x.clone().upcast::<gtk::Widget>())
            .chain(feat_checks.iter().map(|x| x.clone().upcast::<gtk::Widget>()))
            .chain(road_btns.iter().map(|x| x.clone().upcast::<gtk::Widget>()))
        {
            let save = save.clone();
            w.connect_notify_local(Some("active"), move |_, _| save());
        }
        let save = do_save.clone();
        rad_spin.connect_value_changed(move |_| save());
        let save = do_save.clone();
        lvl_spin.connect_value_changed(move |_| save());
        let save = do_save.clone();
        icon_combo.connect_selected_item_notify(move |_| save());
    }

    // === Загрузка данных при выборе другой клетки ===
    let app2 = app.clone();
    let hook: Box<dyn Fn()> = Box::new(move || {
        let data = match app2.selected.get() {
            Some(k) => app2
                .locations
                .borrow()
                .get(&k)
                .cloned()
                .unwrap_or_default(),
            None => LocData::default(),
        };
        loading.set(true);
        for (i, c) in mon_checks.iter().enumerate() {
            c.set_active(data.monsters.contains(&MONSTERS[i].to_string()));
        }
        rad_spin.set_value(data.radiation as f64);
        lvl_spin.set_value(data.monster_level.max(1) as f64);
        for (i, b) in road_btns.iter().enumerate() {
            b.set_active(i == data.road);
        }
        for (i, c) in feat_checks.iter().enumerate() {
            c.set_active(data.features.contains(&FEATURES[i].title.to_string()));
        }
        let idx = ICONS
            .iter()
            .position(|s| *s == data.icon)
            .unwrap_or(0) as u32;
        icon_combo.set_selected(idx);
        loading.set(false);
    });
    append_hook(&app.load_hook, hook);

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&content));
    scroll.upcast()
}

fn section_title(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("title-4");
    l.set_halign(gtk::Align::Start);
    l
}

fn section_title_inline(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_halign(gtk::Align::Start);
    l
}

/// Устанавливает «главный» hook (комментарий), который затем расширяется формой.
fn set_hook(slot: &Hook, f: Box<dyn Fn()>) {
    *slot.borrow_mut() = Some(f);
}

/// Добавляет ещё один обработчик к существующему hook.
fn append_hook(slot: &Hook, extra: Box<dyn Fn()>) {
    let prev = slot.borrow_mut().take();
    let combined: Box<dyn Fn()> = match prev {
        Some(p) => Box::new(move || {
            p();
            extra();
        }),
        None => extra,
    };
    *slot.borrow_mut() = Some(combined);
}

// ---------------------------------------------------------------------------
// Сборка окна
// ---------------------------------------------------------------------------

fn build_ui(state: &Rc<AppState>, app: &adw::Application) -> adw::ApplicationWindow {
    let gtk_app = app.clone().upcast::<gtk::Application>();
    let window = adw::ApplicationWindow::new(&gtk_app);
    window.set_title(Some("TZrMap — TimeZero:Reborn Map"));
    window.set_default_size(1280, 800);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_wide_handle(true);
    paned.set_position(LEFT_WIDTH); // левая половина фиксирована по умолчанию
    paned.set_vexpand(true);

    // ---- Левая часть: сверху мини-карта, снизу tabControl ----
    let left = gtk::Box::new(gtk::Orientation::Vertical, 6);
    left.set_size_request(LEFT_WIDTH, -1);
    left.set_hexpand(false);
    left.set_margin_start(6);
    left.set_margin_end(6);
    left.set_margin_top(6);
    left.set_margin_bottom(6);
    left.append(&setup_minimap(state));

    let notebook = gtk::Notebook::new();
    notebook.set_vexpand(true);
    notebook.append_page(
        &make_comment_tab(state),
        Some(&gtk::Label::new(Some("Комментарий"))),
    );
    notebook.append_page(
        &make_form_tab(state),
        Some(&gtk::Label::new(Some("Локация"))),
    );
    left.append(&notebook);
    paned.set_start_child(Some(&left));

    // ---- Правая часть: карта ----
    paned.set_end_child(Some(&setup_main_map(state)));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&paned);
    root.append(&state.info_label);

    window.set_content(Some(&root));

    // первичная загрузка формы/комментария
    run_hook(&state.load_hook);

    window
}

/// Каталог `img/` рядом с исполняемым файлом (рядом с файлом запуска).
fn img_dir() -> PathBuf {
    let exe_parent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("img")));
    let chosen = match exe_parent {
        Some(ref p) if p.is_dir() => p.clone(),
        _ => PathBuf::from("img"),
    };
    std::fs::create_dir_all(&chosen).ok();
    chosen
}

fn make_state(tiles_dir: PathBuf) -> Rc<AppState> {
    let mut initial_locs: HashMap<(i32, i32), LocData> = HashMap::new();
    // демонстрационная локация: включены все 4 строения (png с альфа-каналом)
    initial_locs.insert(
        (0, 0),
        LocData {
            comment: "Демонстрационная локация".into(),
            features: FEATURES[..4].iter().map(|f| f.title.to_string()).collect(),
            ..LocData::default()
        },
    );
    let state = Rc::new(AppState {
        map: RefCell::new(WorldMap::new()),
        tile_cache: RefCell::new(PngTileCache::new(tiles_dir)),
        selected: Cell::new(None),
        locations: RefCell::new(initial_locs),
        minimap_area: gtk::DrawingArea::new(),
        main_area: gtk::DrawingArea::new(),
        info_label: gtk::Label::default(),
        load_hook: Rc::new(RefCell::new(None)),
    });
    // отступы нижней информационной строки (создаём уже после инициализации GTK,
    // но параметры применим и здесь — Label ещё не «привязан» к дисплею)
    {
        let l = &state.info_label;
        l.set_margin_start(8);
        l.set_margin_end(8);
        l.set_margin_top(4);
        l.set_margin_bottom(4);
        l.set_halign(gtk::Align::Start);
    }

    state
}

/// Отрисовать карту в cairo surface и сохранить png (для проверки без дисплея).
#[cfg(feature = "headless-shot")]
fn headless_shot(state: &Rc<AppState>, path: &Path) {
    let w = 1280;
    let h = 800;
    let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).unwrap();
    let cr = cairo::Context::new(&surf).unwrap();
    // имитируем draw_func основной карты на весь экран
    {
        let area = &state.main_area;
        // вызываем сохранённый draw func нельзя напрямую — рисуем свою копию логики:
        // проще — дёрнем queue_draw невозможно без main context; поэтому повторяем код через helper
        let _ = area;
        draw_main_map_into(state, &cr, w, h);
    }
    let mut file = std::fs::File::create(path).unwrap();
    surf.write_to_png(&mut file).unwrap();
}

fn main() {
    let application = adw::Application::builder()
        .application_id("org.tzrmap.TZrMap")
        .build();

    application.connect_activate(|app| {
        // каталог img/ рядом с исполняемым файлом; базовые картинки
        // создаются автоматически, если пользователь не положил свои
        let tiles_dir = img_dir();
        tiles_gen::ensure_tiles(&tiles_dir);
        let state = make_state(tiles_dir);
        let window = build_ui(&state, app);
        window.present();
    });

    application.run();
}
