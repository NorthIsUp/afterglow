use super::colors::Color;
use save_state::Savable;

pub const LCD_WIDTH: usize = 160;
pub const LCD_HEIGHT: usize = 144;

/// afterglow: pixels are kept as raw RGB555 (`r | g << 5 | b << 10`), so the
/// front end chooses the colour correction, or a DMG palette, itself.
#[derive(Savable)]
pub struct Lcd {
    // x is the only attribute that should be saved, just to be in sync
    // with the PPU rendering, even though the fram will contain half pixels
    // from the state before the load
    x: u8,
    #[savable(skip)]
    buf: Box<[[u16; LCD_WIDTH * LCD_HEIGHT]; 2]>,
    #[savable(skip)]
    selected_buffer: usize,
    /// afterglow: the background layer alone, bit 15 set where the window
    /// drew, and each line's scroll registers as its first pixel left.
    #[savable(skip)]
    bg: Box<[[u16; LCD_WIDTH * LCD_HEIGHT]; 2]>,
    #[savable(skip)]
    scroll: Box<[[[u8; 2]; LCD_HEIGHT]; 2]>,
}

impl Default for Lcd {
    fn default() -> Self {
        Self {
            x: 0,
            buf: Box::new([[0x7FFF; LCD_WIDTH * LCD_HEIGHT]; 2]),
            selected_buffer: 0,
            bg: Box::new([[0x7FFF; LCD_WIDTH * LCD_HEIGHT]; 2]),
            scroll: Box::new([[[0; 2]; LCD_HEIGHT]; 2]),
        }
    }
}

impl Lcd {
    pub fn push(&mut self, color: Color, y: u8) {
        let index = y as usize * LCD_WIDTH + self.x as usize;
        let i = self.next_buffer_index();
        self.buf[i][index] =
            (color.r as u16 & 0x1F) | (color.g as u16 & 0x1F) << 5 | (color.b as u16 & 0x1F) << 10;
        self.x += 1;
    }

    /// Before `push`, which moves `x` on.
    pub fn push_bg(&mut self, color: Color, window: bool, y: u8, scx: u8, scy: u8) {
        let i = self.next_buffer_index();
        if self.x == 0 {
            self.scroll[i][y as usize] = [scx, scy];
        }
        self.bg[i][y as usize * LCD_WIDTH + self.x as usize] = (color.r as u16 & 0x1F)
            | (color.g as u16 & 0x1F) << 5
            | (color.b as u16 & 0x1F) << 10
            | (window as u16) << 15;
    }

    pub fn bg_buffer(&self) -> &[u16] {
        &self.bg[self.selected_buffer]
    }

    pub fn line_scroll(&self) -> &[[u8; 2]] {
        &self.scroll[self.selected_buffer]
    }

    pub fn x(&self) -> u8 {
        self.x
    }

    pub fn next_line(&mut self) {
        self.x = 0;
    }

    pub fn switch_buffers(&mut self) {
        self.selected_buffer = self.next_buffer_index();
    }

    pub fn screen_buffer(&self) -> &[u16] {
        &self.buf[self.selected_buffer]
    }

    pub fn clear(&mut self) {
        for buf in self.buf.iter_mut().chain(self.bg.iter_mut()) {
            buf.fill(0x7FFF);
        }
    }

    pub fn fill(&mut self, color: Color) {
        let saved_x = self.x;
        let saved_selected_buffer = self.selected_buffer;
        self.selected_buffer = self.next_buffer_index();

        self.x = 0;

        for i in 0..LCD_HEIGHT {
            for _j in 0..LCD_WIDTH {
                self.push(color, i as u8)
            }
            self.next_line();
        }

        self.x = saved_x;
        self.selected_buffer = saved_selected_buffer;
    }
}

impl Lcd {
    fn next_buffer_index(&self) -> usize {
        self.selected_buffer ^ 1
    }
}
