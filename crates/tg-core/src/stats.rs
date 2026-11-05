//! Dataset statistics, port of the Python app's Dataset Statistics dock
//! essentials: tagged/untagged counts, tag frequency, token/dimension
//! coverage, aspect-ratio buckets.

use std::collections::HashMap;

use crate::image_entry::ImageEntry;

#[derive(Debug, Clone, Default)]
pub struct DatasetStats {
    pub total_images: usize,
    pub tagged_images: usize,
    pub untagged_images: usize,
    pub total_tags: usize,
    pub average_tags_per_image: f64,
    pub unique_tags: usize,
    pub dimensions_known: usize,
    /// Most common tags, descending by count, up to `top_n`.
    pub top_tags: Vec<(String, usize)>,
    /// Aspect-ratio buckets: `(min_ratio_exclusive, max_ratio_inclusive,
    /// count)`; landscape/square/portrait summary in `ratio_summary`.
    pub ratio_summary: (usize, usize, usize),
}

pub fn compute(images: &[ImageEntry], top_n: usize) -> DatasetStats {
    let mut stats = DatasetStats {
        total_images: images.len(),
        ..Default::default()
    };
    let mut frequency: HashMap<&str, usize> = HashMap::new();
    for image in images {
        if image.tags.is_empty() {
            stats.untagged_images += 1;
        } else {
            stats.tagged_images += 1;
        }
        stats.total_tags += image.tags.len();
        if image.dimensions.is_some() {
            stats.dimensions_known += 1;
        }
        if let Some((width, height)) = image.dimensions {
            if height > 0 {
                let ratio = width as f64 / height as f64;
                if ratio > 1.05 {
                    stats.ratio_summary.0 += 1; // landscape
                } else if ratio < 0.95 {
                    stats.ratio_summary.2 += 1; // portrait
                } else {
                    stats.ratio_summary.1 += 1; // square-ish
                }
            }
        }
        for tag in &image.tags {
            *frequency.entry(tag.as_str()).or_insert(0) += 1;
        }
    }
    stats.average_tags_per_image = if images.is_empty() {
        0.0
    } else {
        stats.total_tags as f64 / images.len() as f64
    };
    stats.unique_tags = frequency.len();
    let mut counts: Vec<(&str, usize)> = frequency.into_iter().collect();
    counts.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    stats.top_tags = counts
        .into_iter()
        .take(top_n)
        .map(|(tag, count)| (tag.to_owned(), count))
        .collect();
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_entry::ImageEntry;
    use std::path::PathBuf;

    fn entry(tags: &[&str], dimensions: Option<(u32, u32)>) -> ImageEntry {
        ImageEntry {
            path: PathBuf::from("/x.png"),
            dimensions,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            file_size: None,
            has_caption_file: true,
            thumbnail: None,
        }
    }

    #[test]
    fn computes_counts_and_frequencies() {
        let images = vec![
            entry(&["1girl", "blue eyes"], Some((800, 600))),
            entry(&["1girl"], Some((600, 800))),
            entry(&[], Some((500, 500))),
        ];
        let stats = compute(&images, 5);
        assert_eq!(stats.total_images, 3);
        assert_eq!(stats.tagged_images, 2);
        assert_eq!(stats.untagged_images, 1);
        assert_eq!(stats.total_tags, 3);
        assert!((stats.average_tags_per_image - 1.0).abs() < 1e-9);
        assert_eq!(stats.unique_tags, 2);
        assert_eq!(stats.top_tags[0], ("1girl".to_string(), 2));
        assert_eq!(stats.top_tags[1], ("blue eyes".to_string(), 1));
        assert_eq!(stats.dimensions_known, 3);
        // landscape, square-ish, portrait
        assert_eq!(stats.ratio_summary, (1, 1, 1));
    }
}
