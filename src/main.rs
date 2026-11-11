#![windows_subsystem = "windows"]

use eframe::egui;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use chrono::{DateTime, Local};
use egui_phosphor::regular;
use sysinfo::Disks;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FO_DELETE, SHFILEOPSTRUCTW};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};
use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP, DeleteObject, GetDIBits, CreateCompatibleDC, DeleteDC, BITMAPINFO, BI_RGB, DIB_RGB_COLORS, BITMAPINFOHEADER};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_MENU, VK_SHIFT, VK_X, VK_A, VK_T, VK_W, VK_N, VK_F2, VK_F5, VK_DELETE, VK_BACK, VK_UP, VK_LEFT, VK_C, VK_V, VK_CONTROL};
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::core::PCWSTR;
use arboard::Clipboard;

#[derive(PartialEq, Clone, Copy)]
enum SortOrder { NameAsc, NameDesc, DateAsc, DateDesc, SizeAsc, SizeDesc }

#[derive(PartialEq, Clone, Copy)]
enum ViewMode { Grid, List }

#[derive(Clone)]
enum DialogAction {
    CreateFolder,
    Rename(PathBuf),
}

#[derive(Clone)]
struct FileItem {
    path: PathBuf,
    name: String,
    is_dir: bool,
    modified: String,
    size: u64,
    size_str: String,
    kind: String,
    icon_key: String,
}

#[derive(PartialEq, Clone, Copy)]
enum TextFieldTarget {
    AddrBuffer,
    SearchQuery,
}

#[derive(Clone)]
struct Tab {
    current_path: PathBuf,
    items: Vec<FileItem>,
    history: Vec<PathBuf>,
    selected_paths: Vec<PathBuf>,
    last_clicked: Option<PathBuf>,
    search_query: String,
    view_mode: ViewMode,
    sort_order: SortOrder,
    rubber_band_start: Option<egui::Pos2>,
    rubber_band_end: Option<egui::Pos2>,
    addr_buffer: String,
    tab_title: String,
    tab_icon: String,
}

impl Tab {
    fn new(path: PathBuf, icon_cache: &mut HashMap<String, egui::TextureHandle>, ctx: &egui::Context) -> Self {
        let (title, icon) = Self::get_title_and_icon(&path);
        let items = Self::read_dir(&path, icon_cache, ctx);
        Self {
            current_path: path.clone(),
            items,
            history: Vec::new(),
            selected_paths: Vec::new(),
            last_clicked: None,
            search_query: String::new(),
            view_mode: ViewMode::Grid,
            sort_order: SortOrder::NameAsc,
            rubber_band_start: None,
            rubber_band_end: None,
            addr_buffer: path.display().to_string(),
            tab_title: title,
            tab_icon: icon,
        }
    }

    fn get_title_and_icon(path: &Path) -> (String, String) {
        if path.as_os_str().is_empty() { return ("Этот компьютер".to_string(), "hdd".to_string()); }
        if let Some(home) = dirs::home_dir() { if path == home { return ("Главная".to_string(), "home".to_string()); } }
        if let Some(desktop) = dirs::desktop_dir() { if path == desktop { return ("Рабочий стол".to_string(), "desktop".to_string()); } }
        if let Some(downloads) = dirs::download_dir() { if path == downloads { return ("Загрузки".to_string(), "downloads".to_string()); } }
        if let Some(docs) = dirs::document_dir() { if path == docs { return ("Документы".to_string(), "documents".to_string()); } }
        if let Some(pics) = dirs::picture_dir() { if path == pics { return ("Изображения".to_string(), "pictures".to_string()); } }
        if let Some(music) = dirs::audio_dir() { if path == music { return ("Музыка".to_string(), "music".to_string()); } }
        if let Some(video) = dirs::video_dir() { if path == video { return ("Видео".to_string(), "video".to_string()); } }
        
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.is_empty() { (path.display().to_string(), "folder".to_string()) } else { (name, "folder".to_string()) }
    }

    fn read_dir(path: &Path, cache: &mut HashMap<String, egui::TextureHandle>, ctx: &egui::Context) -> Vec<FileItem> {
        let mut items = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let meta = match entry.metadata() { Ok(m) => m, Err(_) => continue };
                let modified = meta.modified().unwrap_or(SystemTime::now());
                let dt: DateTime<Local> = modified.into();
                let size = meta.len();
                let size_str = if meta.is_dir() { "--".to_string() } else { format!("{:.1} MB", size as f64 / (1024.0 * 1024.0)) };
                let kind = if meta.is_dir() { "Папка".to_string() } else { entry.path().extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_else(|| "Файл".to_string()) };
                
                let icon_key = RustExplorer::get_icon_texture(&entry.path(), cache, ctx);
                
                items.push(FileItem { 
                    path: entry.path(), 
                    name: entry.file_name().to_string_lossy().into_owned(), 
                    is_dir: meta.is_dir(), 
                    modified: dt.format("%d %b %Y, %H:%M").to_string(), 
                    size, 
                    size_str, 
                    kind,
                    icon_key,
                });
            }
        }
        items
    }

    fn refresh(&mut self, cache: &mut HashMap<String, egui::TextureHandle>, ctx: &egui::Context) {
        if self.current_path.as_os_str().is_empty() {
            self.items.clear();
            return;
        }
        self.items = Self::read_dir(&self.current_path, cache, ctx);
        self.sort_items();
    }

    fn sort_items(&mut self) {
        self.items.sort_by(|a, b| {
            if a.is_dir != b.is_dir { return b.is_dir.cmp(&a.is_dir); }
            match self.sort_order {
                SortOrder::NameAsc => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortOrder::NameDesc => b.name.to_lowercase().cmp(&a.name.to_lowercase()),
                SortOrder::DateAsc => a.modified.cmp(&b.modified),
                SortOrder::DateDesc => b.modified.cmp(&a.modified),
                SortOrder::SizeAsc => a.size.cmp(&b.size),
                SortOrder::SizeDesc => b.size.cmp(&a.size),
            }
        });
    }

    fn navigate_to(&mut self, path: PathBuf, cache: &mut HashMap<String, egui::TextureHandle>, ctx: &egui::Context) {
        if path.is_dir() || path.as_os_str().is_empty() {
            self.history.push(self.current_path.clone());
            self.current_path = path.clone();
            self.addr_buffer = if path.as_os_str().is_empty() { "Этот компьютер".to_string() } else { path.display().to_string() };
            
            let (title, icon) = Self::get_title_and_icon(&path);
            self.tab_title = title;
            self.tab_icon = icon;

            self.refresh(cache, ctx);
            self.selected_paths.clear();
            self.search_query.clear();
        }
    }
}

struct RustExplorer {
    tabs: Vec<Tab>,
    active_tab: usize,
    clipboard_path: Option<PathBuf>,
    is_cut: bool,
    icon_cache: HashMap<String, egui::TextureHandle>,
    ui_icons: HashMap<String, egui::TextureHandle>,
    dialog_action: Option<DialogAction>,
    dialog_input: String,
    dialog_open: bool,
    show_computer_view: bool, 
    
    progress_receiver: Option<Receiver<f32>>,
    progress_value: f32,
    progress_active: bool,
    progress_file_name: String,

    key_states: HashMap<i32, bool>,
    
    context_menu_pos: Option<egui::Pos2>,
    context_menu_target: Option<PathBuf>,

    text_context_menu_pos: Option<egui::Pos2>,
    text_context_menu_target: Option<TextFieldTarget>,
}

impl RustExplorer {
    fn new(ctx: &egui::Context) -> Self {
        let home_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
        
        let mut ui_icons = HashMap::new();
        let icon_data: [(&str, &[u8]); 9] = [
            ("home", include_bytes!("../assets/home.png")),
            ("desktop", include_bytes!("../assets/desktop.png")),
            ("downloads", include_bytes!("../assets/downloads.png")),
            ("documents", include_bytes!("../assets/documents.png")),
            ("pictures", include_bytes!("../assets/pictures.png")),
            ("music", include_bytes!("../assets/music.png")),
            ("video", include_bytes!("../assets/video.png")),
            ("hdd", include_bytes!("../assets/hdd.png")),
            ("folder", include_bytes!("../assets/folder.png")),
        ];

        for (name, bytes) in icon_data {
            if let Ok(image) = image::load_from_memory(bytes) {
                let size = [image.width() as usize, image.height() as usize];
                let image_buffer = image.to_rgba8();
                let pixels = image_buffer.as_flat_samples();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
                let texture = ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR);
                ui_icons.insert(name.to_string(), texture);
            }
        }

        let mut icon_cache = HashMap::new();
        let first_tab = Tab::new(home_path, &mut icon_cache, ctx);

        Self { 
            tabs: vec![first_tab],
            active_tab: 0,
            clipboard_path: None,
            is_cut: false,
            icon_cache,
            ui_icons,
            dialog_action: None,
            dialog_input: String::new(),
            dialog_open: false,
            show_computer_view: false, 
            
            progress_receiver: None,
            progress_value: 0.0,
            progress_active: false,
            progress_file_name: String::new(),
            key_states: HashMap::new(),
            context_menu_pos: None,
            context_menu_target: None,
            text_context_menu_pos: None,
            text_context_menu_target: None,
        }
    }

    fn get_icon_texture(path: &Path, cache: &mut HashMap<String, egui::TextureHandle>, ctx: &egui::Context) -> String {
        let ext = if path.is_dir() { "folder".to_string() } else {
            path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "file".to_string())
        };

        if cache.contains_key(&ext) {
            return ext;
        }

        unsafe {
            let mut path_u16: Vec<u16> = path.as_os_str().encode_wide().collect();
            path_u16.push(0);

            let mut shfi: SHFILEINFOW = std::mem::zeroed();
            let res = SHGetFileInfoW(
                windows::core::PCWSTR(path_u16.as_ptr()),
                FILE_ATTRIBUTE_NORMAL,
                Some(&mut shfi),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            );

            if res != 0 && !shfi.hIcon.is_invalid() {
                let hicon = shfi.hIcon;
                let mut icon_info: ICONINFO = std::mem::zeroed();
                if GetIconInfo(hicon, &mut icon_info).is_ok() {
                    let mut bitmap: BITMAP = std::mem::zeroed();
                    if GetObjectW(icon_info.hbmColor, std::mem::size_of::<BITMAP>() as i32, Some(&mut bitmap as *mut _ as *mut _)) != 0 {
                        let width = bitmap.bmWidth as u32;
                        let height = bitmap.bmHeight as u32;
                        
                        let hdc = CreateCompatibleDC(None);
                        let mut bmi: BITMAPINFO = std::mem::zeroed();
                        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                        bmi.bmiHeader.biWidth = width as i32;
                        bmi.bmiHeader.biHeight = -(height as i32);
                        bmi.bmiHeader.biPlanes = 1;
                        bmi.bmiHeader.biBitCount = 32;
                        bmi.bmiHeader.biCompression = BI_RGB.0;
                        
                        let mut pixels = vec![0u8; (width * height * 4) as usize];
                        GetDIBits(hdc, icon_info.hbmColor, 0, height, Some(pixels.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
                        
                        let mut egui_pixels = Vec::with_capacity((width * height) as usize);
                        for i in (0..pixels.len()).step_by(4) {
                            let b = pixels[i];
                            let g = pixels[i+1];
                            let r = pixels[i+2];
                            let a = pixels[i+3];
                            egui_pixels.push(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
                        }
                        
                        let image = egui::ColorImage { size: [width as usize, height as usize], pixels: egui_pixels };
                        let texture = ctx.load_texture(format!("icon_{}", ext), image, egui::TextureOptions::LINEAR);
                        cache.insert(ext.clone(), texture);
                        
                        let _ = DeleteDC(hdc);
                    }
                    let _ = DeleteObject(icon_info.hbmColor);
                    let _ = DeleteObject(icon_info.hbmMask);
                }
                let _ = DestroyIcon(hicon);
            }
        }
        ext
    }

    fn move_to_recycle_bin(paths: &[PathBuf]) -> bool {
        if paths.is_empty() { return false; }
        
        let mut path_str = Vec::new();
        for p in paths {
            let mut w: Vec<u16> = p.as_os_str().encode_wide().collect();
            path_str.append(&mut w);
            path_str.push(0);
        }
        path_str.push(0);

        let mut file_op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        file_op.wFunc = FO_DELETE;
        file_op.pFrom = PCWSTR(path_str.as_ptr());
        file_op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION).0 as u16;

        unsafe {
            SHFileOperationW(&mut file_op) == 0
        }
    }

    fn paste_file_async(&mut self, ctx: &egui::Context) {
        if let Some(src) = &self.clipboard_path {
            let src = src.clone();
            let src_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
            
            let dest_dir = self.tabs[self.active_tab].current_path.clone();
            let mut dest = dest_dir.join(&src_name);
            
            if dest.exists() {
                let result = rfd::MessageDialog::new()
                    .set_title("Заменить или пропустить файлы")
                    .set_description(format!("В папке назначения уже есть «{}».\n\nЗаменить?", src_name))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                
                if result != rfd::MessageDialogResult::Yes {
                    let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    let mut i = 1;
                    loop {
                        let new_name = format!("{} ({}){}", stem, i, ext);
                        dest = dest_dir.join(&new_name);
                        if !dest.exists() { break; }
                        i += 1;
                    }
                }
            }

            let (tx, rx) = channel();
            self.progress_receiver = Some(rx);
            self.progress_active = true;
            self.progress_value = 0.0;
            self.progress_file_name = dest.file_name().unwrap_or_default().to_string_lossy().into_owned();
            
            let ctx_clone = ctx.clone();
            let is_cut = self.is_cut;
            let is_dir = src.is_dir();

            thread::spawn(move || {
                if is_dir {
                    let _ = Self::copy_dir_all(&src, &dest);
                    let _ = tx.send(1.0);
                    ctx_clone.request_repaint();
                    if is_cut {
                        let _ = fs::remove_dir_all(&src);
                    }
                } else {
                    let result = Self::copy_file_with_progress(&src, &dest, tx, ctx_clone, is_cut);
                    if result.is_ok() && is_cut {
                        let _ = fs::remove_file(&src);
                    }
                }
            });
        }
    }

    fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let new_path = dst.join(entry.file_name());
            if path.is_dir() {
                Self::copy_dir_all(&path, &new_path)?;
            } else {
                fs::copy(&path, &new_path)?;
            }
        }
        Ok(())
    }

    fn copy_file_with_progress(src: &Path, dest: &Path, tx: Sender<f32>, ctx: egui::Context, _is_cut: bool) -> std::io::Result<()> {
        let mut src_file = fs::File::open(src)?;
        let mut dest_file = fs::File::create(dest)?;
        let total_size = src_file.metadata()?.len();
        let mut copied = 0;
        let mut buffer = [0u8; 8192];

        use std::io::{Read, Write};

        loop {
            let n = src_file.read(&mut buffer)?;
            if n == 0 { break; }
            dest_file.write_all(&buffer[..n])?;
            copied += n as u64;
            
            let progress = copied as f32 / total_size as f32;
            let _ = tx.send(progress);
            ctx.request_repaint();
        }
        let _ = tx.send(1.0);
        Ok(())
    }

    fn execute_dialog(&mut self, ctx: &egui::Context) {
        if let Some(action) = &self.dialog_action {
            let name = self.dialog_input.trim();
            if name.is_empty() { return; }

            match action {
                DialogAction::CreateFolder => {
                    let new_path = self.tabs[self.active_tab].current_path.join(name);
                    if !new_path.exists() {
                        let _ = fs::create_dir(&new_path);
                    }
                },
                DialogAction::Rename(path) => {
                    if let Some(parent) = path.parent() {
                        let new_path = parent.join(name);
                        if !new_path.exists() && path != &new_path {
                            let _ = fs::rename(path, &new_path);
                        }
                    }
                }
            }
            self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
            self.dialog_open = false;
            self.dialog_action = None;
        }
    }

    fn create_file_with_ext(&mut self, ctx: &egui::Context, ext: &str) {
        let mut i = 1;
        loop {
            let name = if i == 1 { format!("Новый файл.{}", ext) } else { format!("Новый файл ({}).{}", i, ext) };
            let path = self.tabs[self.active_tab].current_path.join(&name);
            if !path.exists() {
                let _ = fs::write(&path, "");
                self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
                break;
            }
            i += 1;
        }
    }

    fn delete_selected(&mut self, ctx: &egui::Context) {
        let paths = self.tabs[self.active_tab].selected_paths.clone();
        if !paths.is_empty() {
            Self::move_to_recycle_bin(&paths);
        }
        self.tabs[self.active_tab].selected_paths.clear();
        self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
    }

    fn is_key_just_pressed(&mut self, vk: i32) -> bool {
        let is_down = unsafe { (GetAsyncKeyState(vk) & 0x8000u16 as i16) != 0 };
        let was_down = self.key_states.get(&vk).copied().unwrap_or(false);
        self.key_states.insert(vk, is_down);
        is_down && !was_down
    }

    fn is_key_down(&self, vk: i32) -> bool {
        unsafe { (GetAsyncKeyState(vk) & 0x8000u16 as i16) != 0 }
    }
}

impl eframe::App for RustExplorer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.style_mut(|style| {
            style.interaction.selectable_labels = false;
        });

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(32, 32, 32);
        visuals.window_fill = egui::Color32::from_rgb(25, 25, 25);
        visuals.extreme_bg_color = egui::Color32::from_rgb(45, 45, 45);
        visuals.selection.bg_fill = egui::Color32::from_rgb(0, 120, 215);
        
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        
        ctx.set_style(style);
        ctx.set_visuals(visuals);

        // --- ОБРАБОТКА ГОРЯЧИХ КЛАВИШ ---
        let text_edit_has_focus = ctx.memory(|mem| mem.focused().is_some());
        
        if !text_edit_has_focus {
            let ctrl = self.is_key_down(VK_CONTROL.0 as i32);
            let alt = self.is_key_down(VK_MENU.0 as i32);
            let shift = self.is_key_down(VK_SHIFT.0 as i32);

            if self.is_key_just_pressed(VK_C.0 as i32) && ctrl {
                if let Some(first) = self.tabs[self.active_tab].selected_paths.first() {
                    self.clipboard_path = Some(first.clone());
                    self.is_cut = false;
                }
            }
            if self.is_key_just_pressed(VK_X.0 as i32) && ctrl {
                if let Some(first) = self.tabs[self.active_tab].selected_paths.first() {
                    self.clipboard_path = Some(first.clone());
                    self.is_cut = true;
                }
            }
            if self.is_key_just_pressed(VK_V.0 as i32) && ctrl {
                self.paste_file_async(ctx);
            }
            if self.is_key_just_pressed(VK_A.0 as i32) && ctrl {
                let tab = &mut self.tabs[self.active_tab];
                tab.selected_paths = tab.items.iter().map(|i| i.path.clone()).collect();
            }
            if self.is_key_just_pressed(VK_T.0 as i32) && ctrl {
                let new_path = self.tabs[self.active_tab].current_path.clone();
                let new_tab = Tab::new(new_path, &mut self.icon_cache, ctx);
                self.tabs.push(new_tab);
                self.active_tab = self.tabs.len() - 1;
            }
            if self.is_key_just_pressed(VK_W.0 as i32) && ctrl {
                if self.tabs.len() > 1 {
                    self.tabs.remove(self.active_tab);
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len() - 1;
                    }
                }
            }
            if self.is_key_just_pressed(VK_N.0 as i32) && ctrl && shift {
                self.dialog_action = Some(DialogAction::CreateFolder);
                self.dialog_input = "Новая папка".to_string();
                self.dialog_open = true;
            }
            if self.is_key_just_pressed(VK_F2.0 as i32) {
                if let Some(first) = self.tabs[self.active_tab].selected_paths.first() {
                    self.dialog_action = Some(DialogAction::Rename(first.clone()));
                    self.dialog_input = first.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    self.dialog_open = true;
                }
            }
            if self.is_key_just_pressed(VK_DELETE.0 as i32) {
                self.delete_selected(ctx);
            }
            if self.is_key_just_pressed(VK_F5.0 as i32) {
                self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
            }
            if self.is_key_just_pressed(VK_BACK.0 as i32) && !ctrl && !alt {
                let tab = &mut self.tabs[self.active_tab];
                if let Some(prev_path) = tab.history.pop() {
                    tab.current_path = prev_path.clone();
                    tab.addr_buffer = if prev_path.as_os_str().is_empty() { "Этот компьютер".to_string() } else { prev_path.display().to_string() };
                    tab.refresh(&mut self.icon_cache, ctx);
                    tab.selected_paths.clear();
                    tab.search_query.clear();

                    let (title, icon) = Tab::get_title_and_icon(&prev_path);
                    tab.tab_title = title;
                    tab.tab_icon = icon;
                }
            }
            if self.is_key_just_pressed(VK_LEFT.0 as i32) && alt {
                let tab = &mut self.tabs[self.active_tab];
                if let Some(prev_path) = tab.history.pop() {
                    tab.current_path = prev_path.clone();
                    tab.addr_buffer = if prev_path.as_os_str().is_empty() { "Этот компьютер".to_string() } else { prev_path.display().to_string() };
                    tab.refresh(&mut self.icon_cache, ctx);
                    tab.selected_paths.clear();
                    tab.search_query.clear();

                    let (title, icon) = Tab::get_title_and_icon(&prev_path);
                    tab.tab_title = title;
                    tab.tab_icon = icon;
                }
            }
            if self.is_key_just_pressed(VK_UP.0 as i32) && alt {
                let tab = &self.tabs[self.active_tab];
                if let Some(parent) = tab.current_path.parent() {
                    let p = parent.to_path_buf();
                    self.tabs[self.active_tab].navigate_to(p, &mut self.icon_cache, ctx);
                }
            }
        }

        // --- ОКНА ПРОГРЕССА И ДИАЛОГОВ ---
        if self.progress_active {
            if let Some(rx) = &self.progress_receiver {
                if let Ok(progress) = rx.try_recv() {
                    self.progress_value = progress;
                }
            }
            if self.progress_value >= 1.0 {
                self.progress_active = false;
                self.progress_receiver = None;
                self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
            } else {
                egui::Window::new("Копирование...")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        ui.label(&self.progress_file_name);
                        ui.add(egui::ProgressBar::new(self.progress_value).show_percentage());
                    });
                ctx.request_repaint();
            }
        }

        if self.dialog_open {
            egui::Window::new("Введите имя")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label("Имя:");
                    let response = ui.text_edit_singleline(&mut self.dialog_input);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.execute_dialog(ctx);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            self.execute_dialog(ctx);
                        }
                        if ui.button("Отмена").clicked() {
                            self.dialog_open = false;
                            self.dialog_action = None;
                        }
                    });
                });
        }

        // --- КОНТЕКСТНОЕ МЕНЮ (ФАЙЛЫ) ---
        if let Some(pos) = self.context_menu_pos {
            let mut close_menu = false;
            
            if ctx.input(|i| i.pointer.any_click()) {
                if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    let menu_rect = egui::Rect::from_min_size(pos, egui::vec2(200.0, 250.0));
                    if !menu_rect.contains(click_pos) {
                        close_menu = true;
                    }
                }
            }

            egui::Area::new("context_menu".into())
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let frame = egui::Frame::none()
                        .fill(egui::Color32::from_rgb(25, 25, 25))
                        .rounding(12.0)
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 80, 80)))
                        .inner_margin(egui::Margin::symmetric(6.0, 6.0));

                    frame.show(ui, |ui| {
                        ui.set_max_width(200.0);
                        ui.set_min_width(180.0);
                        ui.style_mut().spacing.button_padding = egui::vec2(10.0, 6.0);
                        // УБИРАЕМ СЕРЫЙ ФОН У КНОПОК
                        ui.style_mut().visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                        ui.style_mut().visuals.widgets.hovered.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20);
                        ui.style_mut().visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
                        ui.style_mut().visuals.widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40);
                        
                        let target = self.context_menu_target.clone();
                        
                        if let Some(path) = target {
                            if ui.button(format!("{} Открыть", regular::FOLDER_OPEN)).clicked() {
                                if path.is_dir() { 
                                    self.tabs[self.active_tab].navigate_to(path.clone(), &mut self.icon_cache, ctx); 
                                } else { 
                                    let _ = open::that(&path); 
                                }
                                close_menu = true;
                            }
                            if ui.button(format!("{} Переименовать", regular::PENCIL)).clicked() {
                                self.dialog_action = Some(DialogAction::Rename(path.clone()));
                                self.dialog_input = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                                self.dialog_open = true;
                                close_menu = true;
                            }
                            if ui.button(format!("{} Копировать", regular::COPY)).clicked() {
                                if self.tabs[self.active_tab].selected_paths.is_empty() {
                                    self.clipboard_path = Some(path.clone());
                                } else {
                                    self.clipboard_path = Some(self.tabs[self.active_tab].selected_paths[0].clone());
                                }
                                self.is_cut = false;
                                close_menu = true;
                            }
                            if ui.button(format!("{} Вырезать", regular::SCISSORS)).clicked() {
                                if self.tabs[self.active_tab].selected_paths.is_empty() {
                                    self.clipboard_path = Some(path.clone());
                                } else {
                                    self.clipboard_path = Some(self.tabs[self.active_tab].selected_paths[0].clone());
                                }
                                self.is_cut = true;
                                close_menu = true;
                            }
                            ui.separator();
                            if ui.button(format!("{} Удалить в корзину", regular::TRASH)).clicked() {
                                self.delete_selected(ctx);
                                close_menu = true;
                            }
                        } else {
                            ui.menu_button(format!("{} Создать", regular::FOLDER_PLUS), |ui| {
                                // УБИРАЕМ СЕРЫЙ ФОН И У ВЛОЖЕННОГО МЕНЮ
                                ui.style_mut().visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                                ui.style_mut().visuals.widgets.hovered.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20);
                                
                                if ui.button(format!("{} Папку", regular::FOLDER)).clicked() {
                                    self.dialog_action = Some(DialogAction::CreateFolder);
                                    self.dialog_input = "Новая папка".to_string();
                                    self.dialog_open = true;
                                    close_menu = true;
                                }
                                if ui.button(format!("{} Ярлык", regular::ARROW_SQUARE_OUT)).clicked() {
                                    self.create_file_with_ext(ctx, "lnk");
                                    close_menu = true;
                                }
                                if ui.button(format!("{} Точечный рисунок", regular::IMAGE)).clicked() {
                                    self.create_file_with_ext(ctx, "bmp");
                                    close_menu = true;
                                }
                                if ui.button(format!("{} Текстовый документ", regular::FILE_TEXT)).clicked() {
                                    self.create_file_with_ext(ctx, "txt");
                                    close_menu = true;
                                }
                                if ui.button(format!("{} WinRAR archive", regular::FILE_ARCHIVE)).clicked() {
                                    self.create_file_with_ext(ctx, "rar");
                                    close_menu = true;
                                }
                                if ui.button(format!("{} WinRAR ZIP archive", regular::FILE_ARCHIVE)).clicked() {
                                    self.create_file_with_ext(ctx, "zip");
                                    close_menu = true;
                                }
                            });
                            ui.separator();
                            if ui.button(format!("{} Вставить", regular::CLIPBOARD)).clicked() {
                                self.paste_file_async(ctx);
                                close_menu = true;
                            }
                            if ui.button(format!("{} Обновить", regular::ARROWS_CLOCKWISE)).clicked() {
                                self.tabs[self.active_tab].refresh(&mut self.icon_cache, ctx);
                                close_menu = true;
                            }
                        }
                    });
                });

            if close_menu {
                self.context_menu_pos = None;
                self.context_menu_target = None;
            }
        }

        // --- КОНТЕКСТНОЕ МЕНЮ ДЛЯ ТЕКСТА ---
        if let Some(pos) = self.text_context_menu_pos {
            let mut close_menu = false;
            
            if ctx.input(|i| i.pointer.any_click()) {
                if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                    let menu_rect = egui::Rect::from_min_size(pos, egui::vec2(160.0, 100.0));
                    if !menu_rect.contains(click_pos) {
                        close_menu = true;
                    }
                }
            }

            egui::Area::new("text_context_menu".into())
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let frame = egui::Frame::none()
                        .fill(egui::Color32::from_rgb(25, 25, 25))
                        .rounding(12.0)
                        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 80, 80)))
                        .inner_margin(egui::Margin::symmetric(6.0, 6.0));

                    frame.show(ui, |ui| {
                        ui.set_min_width(150.0);
                        ui.set_max_width(150.0); 
                        ui.style_mut().spacing.button_padding = egui::vec2(10.0, 6.0);
                        // УБИРАЕМ СЕРЫЙ ФОН У КНОПОК
                        ui.style_mut().visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                        ui.style_mut().visuals.widgets.hovered.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20);
                        ui.style_mut().visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
                        ui.style_mut().visuals.widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40);

                        // Кнопка "Копировать"
                        if ui.button(format!("{} Копировать", regular::COPY)).clicked() {
                            let text_to_copy = match self.text_context_menu_target {
                                Some(TextFieldTarget::AddrBuffer) => self.tabs[self.active_tab].addr_buffer.clone(),
                                Some(TextFieldTarget::SearchQuery) => self.tabs[self.active_tab].search_query.clone(),
                                _ => String::new(),
                            };
                            if !text_to_copy.is_empty() {
                                if let Ok(mut clipboard) = Clipboard::new() {
                                    let _ = clipboard.set_text(text_to_copy);
                                }
                            }
                            close_menu = true;
                        }

                        // Кнопка "Вставить"
                        if ui.button(format!("{} Вставить", regular::CLIPBOARD)).clicked() {
                            if let Ok(mut clipboard) = Clipboard::new() {
                                if let Ok(text) = clipboard.get_text() {
                                    match self.text_context_menu_target {
                                        Some(TextFieldTarget::AddrBuffer) => self.tabs[self.active_tab].addr_buffer.push_str(&text),
                                        Some(TextFieldTarget::SearchQuery) => self.tabs[self.active_tab].search_query.push_str(&text),
                                        _ => {}
                                    }
                                }
                            }
                            close_menu = true;
                        }
                    });
                });

            if close_menu {
                self.text_context_menu_pos = None;
                self.text_context_menu_target = None;
            }
        }

        // --- ПАНЕЛЬ ВКЛАДОК ---
        egui::TopBottomPanel::top("tab_bar")
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(10.0, 5.0)).fill(egui::Color32::from_rgb(25, 25, 25)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let mut tab_to_close = None;
                    let mut tab_to_switch = None;
                    let mut tab_to_close_others = None;

                    for (i, tab) in self.tabs.iter().enumerate() {
                        let is_active = i == self.active_tab;
                        let display_name = &tab.tab_title;
                        let icon_name = &tab.tab_icon;

                        let text_width = ui.painter().layout_no_wrap(display_name.clone(), egui::FontId::proportional(13.0), egui::Color32::WHITE).size().x;
                        let tab_width = 16.0 + 4.0 + text_width + 6.0 + 20.0 + 20.0; 

                        let (tab_rect, _) = ui.allocate_exact_size(egui::vec2(tab_width, 28.0), egui::Sense::hover());

                        if is_active {
                            ui.painter().rect_filled(tab_rect, egui::Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(32, 32, 32));
                        }

                        let icon_rect = egui::Rect::from_min_size(tab_rect.min + egui::vec2(10.0, 6.0), egui::vec2(16.0, 16.0));
                        if let Some(tex) = self.ui_icons.get(icon_name) {
                            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                            ui.painter().image(tex.id(), icon_rect, uv, egui::Color32::WHITE);
                        } else {
                            let ico = if icon_name == "hdd" { regular::HARD_DRIVES } else { regular::FOLDER };
                            ui.painter().text(icon_rect.center(), egui::Align2::CENTER_CENTER, ico, egui::FontId::proportional(14.0), egui::Color32::from_rgb(60, 160, 255));
                        }

                        let text_pos = tab_rect.min + egui::vec2(30.0, 14.0);
                        ui.painter().text(text_pos, egui::Align2::LEFT_CENTER, display_name, egui::FontId::proportional(13.0), if is_active { egui::Color32::WHITE } else { egui::Color32::GRAY });

                        let switch_rect = egui::Rect::from_min_max(tab_rect.min, egui::pos2(tab_rect.max.x - 24.0, tab_rect.max.y));
                        let switch_resp = ui.interact(switch_rect, ui.id().with(("switch", i)), egui::Sense::click());
                        
                        if switch_resp.hovered() && !is_active {
                            ui.painter().rect_filled(tab_rect, egui::Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 }, egui::Color32::from_rgb(40, 40, 40));
                        }
                        if switch_resp.clicked() && !is_active {
                            tab_to_switch = Some(i);
                        }

                        let close_rect = egui::Rect::from_min_max(egui::pos2(tab_rect.max.x - 24.0, tab_rect.min.y), tab_rect.max);
                        let close_resp = ui.interact(close_rect, ui.id().with(("close", i)), egui::Sense::click());
                        
                        if close_resp.hovered() {
                            ui.painter().circle_filled(close_rect.center(), 8.0, egui::Color32::from_rgb(60, 60, 60));
                            ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, "✖", egui::FontId::proportional(10.0), egui::Color32::from_rgb(255, 100, 100));
                        } else {
                            ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, "✖", egui::FontId::proportional(10.0), egui::Color32::from_rgb(120, 120, 120));
                        }

                        if close_resp.clicked() {
                            tab_to_close = Some(i);
                        }

                        switch_resp.context_menu(|ui| {
                            if ui.button("Закрыть").clicked() {
                                tab_to_close = Some(i);
                                ui.close_menu();
                            }
                            if ui.button("Закрыть другие").clicked() {
                                tab_to_close_others = Some(i);
                                ui.close_menu();
                            }
                        });
                    }

                    if ui.button("+").clicked() {
                        let new_path = self.tabs[self.active_tab].current_path.clone();
                        let new_tab = Tab::new(new_path, &mut self.icon_cache, ctx);
                        self.tabs.push(new_tab);
                        self.active_tab = self.tabs.len() - 1;
                    }

                    if let Some(idx) = tab_to_switch {
                        self.active_tab = idx;
                    }
                    if let Some(idx) = tab_to_close {
                        if self.tabs.len() > 1 {
                            self.tabs.remove(idx);
                            if self.active_tab >= idx && self.active_tab > 0 {
                                self.active_tab -= 1;
                            }
                        }
                    }
                    if let Some(idx) = tab_to_close_others {
                        let tab_to_keep = self.tabs[idx].clone();
                        self.tabs.clear();
                        self.tabs.push(tab_to_keep);
                        self.active_tab = 0;
                    }
                });
            });

        // --- ВЕРХНЯЯ ПАНЕЛЬ ---
        egui::TopBottomPanel::top("top_bar")
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(15.0, 10.0)).fill(egui::Color32::from_rgb(32, 32, 32)))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(regular::CARET_LEFT).frame(false)).clicked() { 
                    let tab = &mut self.tabs[self.active_tab];
                    if let Some(prev_path) = tab.history.pop() {
                        tab.current_path = prev_path.clone();
                        tab.addr_buffer = if prev_path.as_os_str().is_empty() { "Этот компьютер".to_string() } else { prev_path.display().to_string() };
                        tab.refresh(&mut self.icon_cache, ctx);
                        tab.selected_paths.clear();
                        tab.search_query.clear();

                        let (title, icon) = Tab::get_title_and_icon(&prev_path);
                        tab.tab_title = title;
                        tab.tab_icon = icon;
                    }
                }
                if ui.add(egui::Button::new(regular::CARET_RIGHT).frame(false)).clicked() { }
                ui.add_space(10.0);

                let addr_response = ui.add(
                    egui::TextEdit::singleline(&mut self.tabs[self.active_tab].addr_buffer)
                        .desired_width(350.0)
                        .margin(egui::Margin::symmetric(12.0, 6.0))
                );
                
                if addr_response.clicked_by(egui::PointerButton::Secondary) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        self.text_context_menu_pos = Some(pos);
                        self.text_context_menu_target = Some(TextFieldTarget::AddrBuffer);
                    }
                }

                if addr_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let new_path = PathBuf::from(&self.tabs[self.active_tab].addr_buffer);
                    self.tabs[self.active_tab].navigate_to(new_path, &mut self.icon_cache, ctx);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.tabs[self.active_tab].view_mode == ViewMode::List, regular::LIST).clicked() { self.tabs[self.active_tab].view_mode = ViewMode::List; }
                    if ui.selectable_label(self.tabs[self.active_tab].view_mode == ViewMode::Grid, regular::SQUARES_FOUR).clicked() { self.tabs[self.active_tab].view_mode = ViewMode::Grid; }
                    ui.add_space(10.0);
                    
                    let search_response = ui.add(
                        egui::TextEdit::singleline(&mut self.tabs[self.active_tab].search_query)
                            .hint_text(format!("{} Поиск", regular::MAGNIFYING_GLASS))
                            .desired_width(180.0)
                            .margin(egui::Margin::symmetric(12.0, 6.0))
                    );

                    if search_response.clicked_by(egui::PointerButton::Secondary) {
                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                            self.text_context_menu_pos = Some(pos);
                            self.text_context_menu_target = Some(TextFieldTarget::SearchQuery);
                        }
                    }
                });
            });
        });

        // --- ПАНЕЛЬ ИНСТРУМЕНТОВ ---
        egui::TopBottomPanel::top("action_bar")
            .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(15.0, 2.0)))
            .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button(format!("{} Создать", regular::FOLDER_PLUS), |ui| {
                    if ui.button(format!("{} Папку", regular::FOLDER)).clicked() {
                        self.dialog_action = Some(DialogAction::CreateFolder);
                        self.dialog_input = "Новая папка".to_string();
                        self.dialog_open = true;
                        ui.close_menu();
                    }
                    if ui.button(format!("{} Ярлык", regular::ARROW_SQUARE_OUT)).clicked() {
                        self.create_file_with_ext(ctx, "lnk");
                        ui.close_menu();
                    }
                    if ui.button(format!("{} Точечный рисунок", regular::IMAGE)).clicked() {
                        self.create_file_with_ext(ctx, "bmp");
                        ui.close_menu();
                    }
                    if ui.button(format!("{} Текстовый документ", regular::FILE_TEXT)).clicked() {
                        self.create_file_with_ext(ctx, "txt");
                        ui.close_menu();
                    }
                    if ui.button(format!("{} WinRAR archive", regular::FILE_ARCHIVE)).clicked() {
                        self.create_file_with_ext(ctx, "rar");
                        ui.close_menu();
                    }
                    if ui.button(format!("{} WinRAR ZIP archive", regular::FILE_ARCHIVE)).clicked() {
                        self.create_file_with_ext(ctx, "zip");
                        ui.close_menu();
                    }
                });

                ui.separator();
                if ui.add(egui::Button::new(regular::SCISSORS).frame(false)).clicked() { if let Some(p) = self.tabs[self.active_tab].selected_paths.first() { self.clipboard_path = Some(p.clone()); self.is_cut = true; } }
                if ui.add(egui::Button::new(regular::COPY).frame(false)).clicked() { if let Some(p) = self.tabs[self.active_tab].selected_paths.first() { self.clipboard_path = Some(p.clone()); self.is_cut = false; } }
                if ui.add(egui::Button::new(regular::CLIPBOARD).frame(false)).clicked() { self.paste_file_async(ctx); }
                if ui.add(egui::Button::new(regular::TRASH).frame(false)).clicked() { self.delete_selected(ctx); }
                ui.separator();
                
                ui.menu_button(format!("{} Сортировать", regular::SORT_ASCENDING), |ui| {
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::NameAsc, "Имя (А-Я)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::NameAsc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::NameDesc, "Имя (Я-А)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::NameDesc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::DateAsc, "Дата (Сначала старые)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::DateAsc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::DateDesc, "Дата (Сначала новые)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::DateDesc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::SizeAsc, "Размер (Сначала маленькие)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::SizeAsc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                    if ui.selectable_label(self.tabs[self.active_tab].sort_order == SortOrder::SizeDesc, "Размер (Сначала большие)").clicked() { 
                        self.tabs[self.active_tab].sort_order = SortOrder::SizeDesc; 
                        self.tabs[self.active_tab].sort_items(); 
                    }
                });
            });
        });

        // --- ЛЕВАЯ ПАНЕЛЬ ---
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(220.0)
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(28, 28, 28)).inner_margin(egui::Margin::symmetric(10.0, 10.0)))
            .show(ctx, |ui| {
                let mut to_navigate: Option<PathBuf> = None;
                let mut is_computer_clicked = false;

                let mut menu_item = |ui: &mut egui::Ui, icon_name: &str, text: &str, is_selected: bool, path: &Option<PathBuf>| {
                    let mut frame = egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(10.0, 6.0)) 
                        .rounding(6.0);
                    
                    if is_selected { 
                        frame = frame.fill(egui::Color32::from_rgba_unmultiplied(0, 120, 215, 50)); 
                    }

                    let response = frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if let Some(tex) = self.ui_icons.get(icon_name) {
                                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(18.0, 18.0)));
                            } else {
                                ui.label("📄");
                            }
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(text).size(13.0).color(egui::Color32::WHITE));
                        });
                    }).response;
                    
                    if response.interact(egui::Sense::click()).clicked() {
                        to_navigate = path.clone();
                    }
                };

                ui.label(egui::RichText::new("Избранное").size(11.0).color(egui::Color32::GRAY));
                ui.add_space(4.0);
                
                let home = dirs::home_dir();
                menu_item(ui, "home", "Главная", home.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &home);
                
                let desktop = dirs::desktop_dir();
                menu_item(ui, "desktop", "Рабочий стол", desktop.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &desktop);
                
                let downloads = dirs::download_dir();
                menu_item(ui, "downloads", "Загрузки", downloads.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &downloads);
                
                let docs = dirs::document_dir();
                menu_item(ui, "documents", "Документы", docs.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &docs);
                
                let pics = dirs::picture_dir();
                menu_item(ui, "pictures", "Изображения", pics.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &pics);
                
                let music = dirs::audio_dir();
                menu_item(ui, "music", "Музыка", music.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &music);
                
                let video = dirs::video_dir();
                menu_item(ui, "video", "Видео", video.as_ref().map_or(false, |p| self.tabs[self.active_tab].current_path == *p), &video);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                
                let is_pc_selected = self.show_computer_view;
                let id = ui.make_persistent_id("pc_tree");
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
                
                let header_res = ui.horizontal(|ui| {
                    let (arrow_rect, arrow_resp) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
                    if arrow_resp.clicked() {
                        state.toggle(ui);
                    }
                    let center = arrow_rect.center();
                    let points = if state.is_open() {
                        vec![egui::pos2(center.x - 4.0, center.y - 2.0), egui::pos2(center.x + 4.0, center.y - 2.0), egui::pos2(center.x, center.y + 4.0)]
                    } else {
                        vec![egui::pos2(center.x - 2.0, center.y - 4.0), egui::pos2(center.x - 2.0, center.y + 4.0), egui::pos2(center.x + 4.0, center.y)]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(points, egui::Color32::GRAY, egui::Stroke::NONE));
                    
                    ui.add_space(4.0);

                    if let Some(tex) = self.ui_icons.get("hdd") {
                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(18.0, 18.0)));
                    } else {
                        ui.label(egui::RichText::new(regular::HARD_DRIVES).color(egui::Color32::from_rgb(60, 160, 255)).size(16.0));
                    }
                    ui.add_space(8.0);

                    let text_color = if is_pc_selected { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 200, 200) };
                    let text_resp = ui.add(egui::Label::new(egui::RichText::new("Этот компьютер").size(13.0).color(text_color)).sense(egui::Sense::click()));
                    if text_resp.clicked() {
                        is_computer_clicked = true;
                    }
                }).response;

                if header_res.interact(egui::Sense::click()).clicked() {
                    is_computer_clicked = true;
                }
                
                state.store(ui.ctx());

                if state.is_open() {
                    let disks = Disks::new_with_refreshed_list();
                    for disk in &disks {
                        let mount_point = disk.mount_point().to_string_lossy().to_string();
                        let drive_path = Some(disk.mount_point().to_path_buf());
                        let is_selected = self.tabs[self.active_tab].current_path == disk.mount_point();
                        
                        ui.horizontal(|ui| {
                            ui.add_space(24.0); 
                            
                            let mut frame = egui::Frame::none().inner_margin(egui::Margin::symmetric(10.0, 6.0)).rounding(6.0);
                            if is_selected { frame = frame.fill(egui::Color32::from_rgba_unmultiplied(0, 120, 215, 50)); }
                            
                            let response = frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if let Some(tex) = self.ui_icons.get("hdd") {
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(16.0, 16.0)));
                                    } else {
                                        ui.label(egui::RichText::new(regular::HARD_DRIVES).color(egui::Color32::from_rgb(150, 150, 150)).size(14.0));
                                    }
                                    ui.add_space(8.0);
                                    ui.label(egui::RichText::new(format!("Диск {}", mount_point)).size(13.0).color(egui::Color32::WHITE));
                                });
                            }).response;
                            
                            if response.interact(egui::Sense::click()).clicked() {
                                to_navigate = drive_path;
                            }
                        });
                    }
                }

                if is_computer_clicked {
                    self.show_computer_view = true;
                    self.tabs[self.active_tab].navigate_to(PathBuf::from(""), &mut self.icon_cache, ctx);
                } else if let Some(path) = to_navigate {
                    self.show_computer_view = false;
                    self.tabs[self.active_tab].navigate_to(path, &mut self.icon_cache, ctx);
                }
            });

        // --- ЦЕНТРАЛЬНАЯ ПАНЕЛЬ ---
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(25, 25, 25)).inner_margin(egui::Margin::symmetric(20.0, 20.0)))
            .show(ctx, |ui| {
            
            let query = self.tabs[self.active_tab].search_query.to_lowercase();
            let filtered_items: Vec<FileItem> = self.tabs[self.active_tab].items.iter().filter(|item| item.name.to_lowercase().contains(&query)).map(|item| item.clone()).collect();
            let current_path = self.tabs[self.active_tab].current_path.clone();
            let view_mode = self.tabs[self.active_tab].view_mode;

            let bg_response = ui.interact(ui.available_rect_before_wrap(), ui.id().with("bg_area"), egui::Sense::click_and_drag());
            
            if bg_response.clicked_by(egui::PointerButton::Secondary) {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    self.context_menu_pos = Some(pos);
                    self.context_menu_target = None;
                }
            }

            if bg_response.clicked() { self.tabs[self.active_tab].selected_paths.clear(); self.tabs[self.active_tab].last_clicked = None; }
            if bg_response.drag_started() { 
                let pos = bg_response.interact_pointer_pos().unwrap();
                self.tabs[self.active_tab].rubber_band_start = Some(pos); 
                self.tabs[self.active_tab].rubber_band_end = Some(pos);
                self.tabs[self.active_tab].selected_paths.clear();
            }

            let mut to_navigate: Option<PathBuf> = None;
            let action_rename: Option<PathBuf> = None;

            let is_computer_view = self.show_computer_view; 

            egui::ScrollArea::vertical().show(ui, |ui| {
                if is_computer_view {
                     ui.add_space(5.0);
                     ui.label(egui::RichText::new("Папки").size(15.0).strong().color(egui::Color32::WHITE));
                     ui.add_space(10.0);
                     
                     ui.horizontal_wrapped(|ui| {
                         let folders = [
                             ("Рабочий стол", "desktop", dirs::desktop_dir()),
                             ("Загрузки", "downloads", dirs::download_dir()),
                             ("Документы", "documents", dirs::document_dir()),
                             ("Изображения", "pictures", dirs::picture_dir()),
                             ("Музыка", "music", dirs::audio_dir()),
                             ("Видео", "video", dirs::video_dir()),
                         ];

                         for (name, icon_name, path) in folders {
                             let card_size = egui::vec2(110.0, 120.0);
                             let (rect, resp) = ui.allocate_exact_size(card_size, egui::Sense::click());
                             
                             let bg_color = if resp.hovered() { egui::Color32::from_rgb(50, 50, 50) } else { egui::Color32::TRANSPARENT };
                             ui.painter().rect_filled(rect, 10.0, bg_color);

                             let icon_rect = egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 40.0), egui::vec2(48.0, 48.0));
                             if let Some(tex) = self.ui_icons.get(icon_name) {
                                 let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                 ui.painter().image(tex.id(), icon_rect, uv, egui::Color32::WHITE);
                             }

                             ui.painter().text(rect.center_bottom() - egui::vec2(0.0, 15.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(12.0), egui::Color32::WHITE);

                             if resp.clicked() { if let Some(p) = path { to_navigate = Some(p); } }
                         }
                     });

                     ui.add_space(20.0);
                     ui.label(egui::RichText::new("Устройства и диски").size(15.0).strong().color(egui::Color32::WHITE));
                     ui.add_space(10.0);

                     let disks = Disks::new_with_refreshed_list();
                     for disk in &disks {
                         let (rect, resp) = ui.allocate_exact_size(egui::vec2(400.0, 70.0), egui::Sense::click());
                         
                         let bg_color = if resp.hovered() { egui::Color32::from_rgb(50, 50, 50) } else { egui::Color32::from_rgb(40, 40, 40) };
                         ui.painter().rect_filled(rect, 10.0, bg_color);
                         
                         let icon_rect = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 10.0), egui::vec2(50.0, 50.0));
                         if let Some(tex) = self.ui_icons.get("hdd") {
                             let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                             ui.painter().image(tex.id(), icon_rect, uv, egui::Color32::WHITE);
                         } else {
                             ui.painter().rect_filled(icon_rect, 8.0, egui::Color32::from_rgb(60, 160, 255));
                             ui.painter().text(icon_rect.center(), egui::Align2::CENTER_CENTER, regular::HARD_DRIVES, egui::FontId::proportional(24.0), egui::Color32::WHITE);
                         }

                         let name = format!("Локальный диск ({})", disk.mount_point().display());
                         ui.painter().text(rect.min + egui::vec2(70.0, 15.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(13.0), egui::Color32::WHITE);
                         
                         let total = disk.total_space();
                         let free = disk.available_space();
                         let used = total - free;
                         let ratio = if total > 0 { used as f32 / total as f32 } else { 0.0 };

                         let bar_rect = egui::Rect::from_min_size(rect.min + egui::vec2(70.0, 35.0), egui::vec2(250.0, 6.0));
                         ui.painter().rect_filled(bar_rect, 3.0, egui::Color32::from_rgb(80, 80, 80));
                         let mut fill_rect = bar_rect; fill_rect.set_width(250.0 * ratio); 
                         ui.painter().rect_filled(fill_rect, 3.0, egui::Color32::from_rgb(60, 160, 255));
                         
                         let free_gb = free as f64 / (1024.0 * 1024.0 * 1024.0);
                         let total_gb = total as f64 / (1024.0 * 1024.0 * 1024.0);
                         ui.painter().text(rect.min + egui::vec2(70.0, 45.0), egui::Align2::LEFT_TOP, format!("{:.1} ГБ свободно из {:.1} ГБ", free_gb, total_gb), egui::FontId::proportional(10.0), egui::Color32::GRAY);
                         
                         if resp.clicked() || resp.double_clicked() {
                             to_navigate = Some(disk.mount_point().to_path_buf());
                         }
                         ui.add_space(8.0);
                     }
                } else {
                    if view_mode == ViewMode::List {
                        let col_widths = [ui.available_width() - 350.0, 200.0, 80.0, 80.0];
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.allocate_ui(egui::vec2(col_widths[0], 20.0), |ui| { ui.label(egui::RichText::new("Имя").size(11.0).strong().color(egui::Color32::GRAY)); });
                            ui.allocate_ui(egui::vec2(col_widths[1], 20.0), |ui| { ui.label(egui::RichText::new("Дата").size(11.0).strong().color(egui::Color32::GRAY)); });
                            ui.allocate_ui(egui::vec2(col_widths[2], 20.0), |ui| { ui.label(egui::RichText::new("Размер").size(11.0).strong().color(egui::Color32::GRAY)); });
                            ui.allocate_ui(egui::vec2(col_widths[3], 20.0), |ui| { ui.label(egui::RichText::new("Тип").size(11.0).strong().color(egui::Color32::GRAY)); });
                        });

                        for item in &filtered_items {
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click_and_drag());
                            
                            if response.clicked_by(egui::PointerButton::Secondary) {
                                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                    self.context_menu_pos = Some(pos);
                                    self.context_menu_target = Some(item.path.clone());
                                    if !self.tabs[self.active_tab].selected_paths.contains(&item.path) {
                                        self.tabs[self.active_tab].selected_paths = vec![item.path.clone()];
                                    }
                                }
                            }

                            if self.tabs[self.active_tab].rubber_band_start.is_some() {
                                if let (Some(start), Some(end)) = (self.tabs[self.active_tab].rubber_band_start, self.tabs[self.active_tab].rubber_band_end) {
                                    let band_rect = egui::Rect::from_two_pos(start, end);
                                    if band_rect.intersects(rect) {
                                        if !self.tabs[self.active_tab].selected_paths.contains(&item.path) {
                                            self.tabs[self.active_tab].selected_paths.push(item.path.clone());
                                        }
                                    } else {
                                        self.tabs[self.active_tab].selected_paths.retain(|p| p != &item.path);
                                    }
                                }
                            }

                            let is_selected = self.tabs[self.active_tab].selected_paths.contains(&item.path);
                            if is_selected { ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 120, 215, 100)); }
                            else if response.hovered() { ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgb(50, 50, 50)); }

                            ui.allocate_ui_at_rect(rect, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(10.0);
                                    if let Some(tex) = self.icon_cache.get(&item.icon_key) {
                                        let tint = if self.is_cut && Some(&item.path) == self.clipboard_path.as_ref() { egui::Color32::from_white_alpha(100) } else { egui::Color32::WHITE };
                                        ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(24.0, 24.0)).tint(tint));
                                        ui.add_space(5.0);
                                    } else {
                                        let icon = if item.is_dir { regular::FOLDER } else { regular::FILE };
                                        let color = if item.is_dir { egui::Color32::from_rgb(60, 160, 255) } else { egui::Color32::GRAY };
                                        ui.label(egui::RichText::new(icon).color(color).size(24.0));
                                        ui.add_space(5.0);
                                    }
                                    
                                    ui.allocate_ui(egui::vec2(col_widths[0] - 35.0, 32.0), |ui| { ui.label(egui::RichText::new(&item.name).color(egui::Color32::WHITE)); });
                                    ui.allocate_ui(egui::vec2(col_widths[1], 32.0), |ui| { ui.label(egui::RichText::new(&item.modified).color(egui::Color32::GRAY)); });
                                    ui.allocate_ui(egui::vec2(col_widths[2], 32.0), |ui| { ui.label(egui::RichText::new(&item.size_str).color(egui::Color32::GRAY)); });
                                    ui.allocate_ui(egui::vec2(col_widths[3], 32.0), |ui| { ui.label(egui::RichText::new(&item.kind).color(egui::Color32::GRAY)); });
                                });
                            });

                            if response.clicked() { self.tabs[self.active_tab].selected_paths = vec![item.path.clone()]; self.tabs[self.active_tab].last_clicked = Some(item.path.clone()); }
                            if response.double_clicked() { if item.is_dir { to_navigate = Some(item.path.clone()); } else { let _ = open::that(&item.path); } }
                        }
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            for item in &filtered_items {
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(110.0, 130.0), egui::Sense::click());
                                
                                if resp.clicked_by(egui::PointerButton::Secondary) {
                                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                        self.context_menu_pos = Some(pos);
                                        self.context_menu_target = Some(item.path.clone());
                                        if !self.tabs[self.active_tab].selected_paths.contains(&item.path) {
                                            self.tabs[self.active_tab].selected_paths = vec![item.path.clone()];
                                        }
                                    }
                                }

                                if self.tabs[self.active_tab].rubber_band_start.is_some() {
                                    if let (Some(start), Some(end)) = (self.tabs[self.active_tab].rubber_band_start, self.tabs[self.active_tab].rubber_band_end) {
                                        let band_rect = egui::Rect::from_two_pos(start, end);
                                        if band_rect.intersects(rect) {
                                            if !self.tabs[self.active_tab].selected_paths.contains(&item.path) {
                                                self.tabs[self.active_tab].selected_paths.push(item.path.clone());
                                            }
                                        } else {
                                            self.tabs[self.active_tab].selected_paths.retain(|p| p != &item.path);
                                        }
                                    }
                                }

                                let is_selected = self.tabs[self.active_tab].selected_paths.contains(&item.path);
                                let bg_color = if is_selected { egui::Color32::from_rgba_unmultiplied(0, 120, 215, 100) } 
                                               else if resp.hovered() { egui::Color32::from_rgb(50, 50, 50) } 
                                               else { egui::Color32::TRANSPARENT };
                                ui.painter().rect_filled(rect, 10.0, bg_color);

                                let icon_rect = egui::Rect::from_center_size(rect.center_top() + egui::vec2(0.0, 45.0), egui::vec2(56.0, 56.0));
                                if let Some(tex) = self.icon_cache.get(&item.icon_key) {
                                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                    let tint = if self.is_cut && Some(&item.path) == self.clipboard_path.as_ref() { egui::Color32::from_white_alpha(100) } else { egui::Color32::WHITE };
                                    ui.painter().image(tex.id(), icon_rect, uv, tint);
                                } else {
                                    let icon = if item.is_dir { regular::FOLDER } else { regular::FILE };
                                    let color = if item.is_dir { egui::Color32::from_rgb(60, 160, 255) } else { egui::Color32::GRAY };
                                    ui.painter().text(icon_rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(40.0), color);
                                }
                                
                                let max_chars = 14;
                                let display_name = if item.name.chars().count() > max_chars { format!("{}...", item.name.chars().take(max_chars).collect::<String>()) } else { item.name.clone() };
                                
                                let text_pos = rect.center_top() + egui::vec2(0.0, 85.0);
                                ui.painter().text(text_pos, egui::Align2::CENTER_CENTER, display_name, egui::FontId::proportional(12.0), egui::Color32::WHITE);

                                if resp.clicked() { self.tabs[self.active_tab].selected_paths = vec![item.path.clone()]; self.tabs[self.active_tab].last_clicked = Some(item.path.clone()); }
                                if resp.double_clicked() { if item.is_dir { to_navigate = Some(item.path.clone()); } else { let _ = open::that(&item.path); } }
                            }
                        });
                    }
                }
            });

            if let Some(start_pos) = self.tabs[self.active_tab].rubber_band_start {
                if let Some(current_pos) = ui.input(|i| i.pointer.hover_pos()) {
                    self.tabs[self.active_tab].rubber_band_end = Some(current_pos);
                    let rect = egui::Rect::from_two_pos(start_pos, current_pos);
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(0, 120, 215, 50));
                    ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(0, 120, 215)));
                }
            }

            if bg_response.drag_stopped() { 
                self.tabs[self.active_tab].rubber_band_start = None; 
                self.tabs[self.active_tab].rubber_band_end = None;
            }

            if let Some(path) = action_rename {
                self.dialog_action = Some(DialogAction::Rename(path.clone()));
                self.dialog_input = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                self.dialog_open = true;
            }

            if let Some(path) = to_navigate { 
                self.show_computer_view = false; 
                self.tabs[self.active_tab].navigate_to(path, &mut self.icon_cache, ctx);
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 750.0])
            .with_title("Rust Explorer"),
        ..Default::default()
    };
    eframe::run_native("Rust Explorer", options, Box::new(|cc| {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        Box::new(RustExplorer::new(&cc.egui_ctx))
    }))
}