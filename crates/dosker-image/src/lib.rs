//! Image references, FAT image access, and the local image store.

pub mod error;
pub mod fat;
pub mod image;
pub mod reference;
pub mod store;

pub use error::ImageError;
pub use fat::{
    CONFIG_DIR, CONFIG_FILE, CONFIG_PATH, DEFAULT_BAUD, DEFAULT_CONTROL_PORT, FatLayout, FatRegion,
    fat_region, read_file, write_config,
};
pub use image::{ImageMedia, ImageRecord};
pub use reference::{ImageName, ImageReference, ImageTag};
pub use store::{IMAGE_DISK_FILE, IMAGE_RECORD_FILE, ImageStore};
