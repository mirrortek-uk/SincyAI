//! Unified pagination (TDD 12.1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequest {
    /// 1-based page number.
    #[serde(default = "default_page")]
    pub page: u32,
    /// Items per page, capped by `MAX_PAGE_SIZE`.
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

pub const MAX_PAGE_SIZE: u32 = 200;

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    20
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            page: default_page(),
            page_size: default_page_size(),
        }
    }
}

impl PageRequest {
    pub fn offset(&self) -> u64 {
        (self.page.saturating_sub(1) as u64) * (self.page_size() as u64)
    }
    pub fn limit(&self) -> u64 {
        self.page_size().into()
    }
    fn page_size(&self) -> u32 {
        self.page_size.clamp(1, MAX_PAGE_SIZE)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, req: &PageRequest, total: u64) -> Self {
        Self {
            items,
            page: req.page,
            page_size: req.page_size,
            total,
        }
    }
    pub fn total_pages(&self) -> u64 {
        let ps = self.page_size.max(1) as u64;
        self.total.div_ceil(ps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_is_capped() {
        let req = PageRequest {
            page: 2,
            page_size: 10_000,
        };
        assert_eq!(req.limit(), MAX_PAGE_SIZE as u64);
        assert_eq!(req.offset(), MAX_PAGE_SIZE as u64);
    }

    #[test]
    fn total_pages_ceil() {
        let req = PageRequest::default();
        let page: Page<u32> = Page::new(vec![1, 2], &req, 41);
        assert_eq!(page.total_pages(), 3);
    }
}
