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
//!    отображается png-изображением из каталога `tiles/` (или заливкой по типу).

mod terrain;
mod tiles_gen;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};
use libadwaita::{self as adw, prelude::*};

use terrain::{Terrain, TileCache, TilePos, WorldMap};

/// Размер клетки основной карты в пикселях.
const TILE_PX: i32 = 48;
/// Ширина мини-карты в пикселях (1 пиксель = 1 тайл, всего 361x361).
const MINIMAP_SIZE: i32 = 361;
/// Ширина левой панели по умолчанию.
const LEFT_WIDTH: i32 = 420;

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

const FEATURES: [&str; 6] = [
    "Торговцы",
    "Магазины",
    "Квесты",
    "Аномалии",
    "Погодные аномалии",
    "Подземелье",
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
    tile_cache: RefCell<TileCache>,
    selected: Cell<Option<(i32, i32)>>,
    locations: RefCell<HashMap<(i32, i32), LocData>>,
    minimap_area: gtk::DrawingArea,
    main_area: gtk::DrawingArea,
    info_label: gtk::Label,
    /// вызывается при смене выбранной клетки — обновляет обе вкладки
    load_hook: Hook,
}

impl AppState {
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
        self.minimap_area.queue_draw();
        self.main_area.queue_draw();
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
    c.set_child(Some(&gtk::Label::new(Some(label))));
    c
}

/// ToggleButton с текстовой меткой.
fn toggle_button(label: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&gtk::Label::new(Some(label))));
    b
}

/// RadioButton в группе `group` с меткой.
fn radio_button(group: Option<&gtk::RadioButton>, label: &str) -> gtk::RadioButton {
    match group {
        Some(g) => gtk::RadioButton::builder().label(label).group(g).build(),
        None => gtk::RadioButton::builder().label(label).build(),
    }
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
            cr.paint();
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
                    cr.fill();
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
                cr.stroke();
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

    let frame = gtk::Frame::new(Some(area));
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
        let cols = (w / TILE_PX).clamp(1, 361);
        let rows = (h / TILE_PX).clamp(1, 361);
        for row in 0..rows {
            for col in 0..cols {
                let lon = -180 + col;
                let lat = 180 - row;
                let terrain = map.get(lon, lat);
                let x = (col * TILE_PX) as f64;
                let y = (row * TILE_PX) as f64;

                // png-картинка локации, если есть
                let path = app2
                    .tile_cache
                    .borrow_mut()
                    .tile_path(TilePos { lon, lat }, terrain)
                    .map(|p| p.to_path_buf());
                let mut drawn = false;
                if let Some(p) = path {
                    if let Ok(tex) = gdk::Texture::for_filename(&p) {
                        cr.set_texture(&tex);
                        cr.save();
                        cr.translate(x, y);
                        cr.scale(
                            TILE_PX as f64 / tex.width() as f64,
                            TILE_PX as f64 / tex.height() as f64,
                        );
                        cr.paint();
                        cr.restore();
                        drawn = true;
                    }
                }
                if !drawn {
                    let (r, g, b) = terrain.color();
                    cr.set_source_rgb(r as f64 / 255., g as f64 / 255., b as f64 / 255.);
                    cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
                    cr.fill();
                }
                // тонкая сетка тайлового поля
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.15);
                cr.set_line_width(1.0);
                cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
                cr.stroke();
            }
        }
        // подсветка выбранной клетки
        if let Some((lon, lat)) = app2.selected.get() {
            let x = ((lon + 180) * TILE_PX) as f64;
            let y = ((180 - lat) * TILE_PX) as f64;
            cr.set_source_rgba(1.0, 0.1, 0.1, 0.95);
            cr.set_line_width(3.0);
            cr.rectangle(x, y, TILE_PX as f64, TILE_PX as f64);
            cr.stroke();
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

    let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
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
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_vexpand(true);

    let view = gtk::TextView::new();
    view.set_wrap_mode(pango::WrapMode::WordChar);
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
            let text = buf.text(&buf.start_mark(), &buf.end_mark(), true);
            state.locations.borrow_mut().entry(key).or_default().comment = text;
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
        .map(|(i, name)| {
            let c = check_button(name);
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
    let icon_combo = gtk::DropDown::from_model(Some(&icon_model), 0);
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
                .map(|(_, f)| (*f).to_string())
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
            .map(|x| x.as_widget())
            .chain(feat_checks.iter().map(|x| x.as_widget()))
            .chain(road_btns.iter().map(|x| x.as_widget()))
        {
            w.connect_notify(Some("active"), move |_, _| save());
        }
        let save = do_save.clone();
        rad_spin.connect_value_changed(move |_| save());
        let save = do_save.clone();
        lvl_spin.connect_value_changed(move |_| save());
        let save = do_save.clone();
        icon_combo.connect_selected_items_notify(move |_| save());
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
            c.set_active(data.features.contains(&FEATURES[i].to_string()));
        }
        let idx = ICONS
            .iter()
            .position(|s| *s == data.icon)
            .unwrap_or(0) as u32;
        icon_combo.set_selected(idx);
        loading.set(false);
    });
    append_hook(&app.load_hook, hook);

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
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

fn build_ui(app: &Rc<AppState>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::new();
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
    left.append(&setup_minimap(app));

    let notebook = gtk::Notebook::new();
    notebook.set_vexpand(true);
    notebook.append_page(
        &make_comment_tab(app),
        Some(&gtk::Label::new(Some("Комментарий"))),
    );
    notebook.append_page(
        &make_form_tab(app),
        Some(&gtk::Label::new(Some("Локация"))),
    );
    left.append(&notebook);
    paned.set_start_child(Some(&left));

    // ---- Правая часть: карта ----
    paned.set_end_child(Some(&setup_main_map(app)));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&paned);
    root.append(&app.info_label);

    window.set_content(Some(&root));

    // первичная загрузка формы/комментария
    run_hook(&app.load_hook);

    window
}

fn main() {
    adw::init().expect("Не удалось инициализировать libadwaita");

    // каталог с png-тайлами локаций; базовые картинки создаются автоматически
    let tiles_dir = std::env::var("TZRMAP_TILES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("tiles")
        });
    tiles_gen::ensure_tiles(&tiles_dir);

    let info_label = gtk::Label::new(Some(""));
    info_label.set_margin_start(8);
    info_label.set_margin_end(8);
    info_label.set_margin_top(4);
    info_label.set_margin_bottom(4);
    info_label.set_halign(gtk::Align::Start);

    let app = Rc::new(AppState {
        map: RefCell::new(WorldMap::new()),
        tile_cache: RefCell::new(TileCache::new(&tiles_dir)),
        selected: Cell::new(None),
        locations: RefCell::new(HashMap::new()),
        minimap_area: gtk::DrawingArea::new(),
        main_area: gtk::DrawingArea::new(),
        info_label,
        load_hook: Rc::new(RefCell::new(None)),
    });

    let window = build_ui(&app);

    // простой событийный цикл без GtkApplication: завершаем работу при закрытии окна
    let running = Rc::new(Cell::new(true));
    {
        let running = running.clone();
        window.connect_close_requested(move |_| {
            running.set(false);
            glib::Propagation::Proceed
        });
    }
    window.present();

    let ctx = glib::MainContext::default();
    while running.get() {
        ctx.iteration(true);
    }
}
