//! WD Tagger via ONNX Runtime. Port of
//! `taggui/auto_captioning/models/wd_tagger.py`: preprocessing (white
//! composite -> square pad -> bicubic resize -> RGB -> BGR), tag CSV
//! parsing (kaomojis kept verbatim), rating-tag exclusion, thresholding,
//! sorting, and truncation.

use std::path::Path;

use ndarray::Array4;

/// Kaomojis are not underscore-replaced (from `wd_tagger.py::KAOMOJIS`).
pub const KAOMOJIS: [&str; 19] = [
    "0_0", "(o)_(o)", "+_+", "+_-", "._.", "<o>_<o>", "<|>_<|>", "=_=", ">_<", "3_3", "6_9", ">_o",
    "@_@", "^_^", "o_o", "u_u", "x_x", "|_|", "||_||",
];

#[derive(Debug, thiserror::Error)]
pub enum WdTaggerError {
    #[error("failed to read the image {path}: {source}")]
    Image {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("failed to load the ONNX model: {0}")]
    Model(String),
    #[error("inference failed: {0}")]
    Inference(String),
    #[error("failed to read the tag list {path}: {source}")]
    Tags {
        path: PathBuf,
        source: std::io::Error,
    },
}

use std::path::PathBuf;

/// One entry of `selected_tags.csv`.
#[derive(Debug, Clone)]
pub struct TagEntry {
    pub name: String,
    pub category: i32, // 9 = rating, 0 = general, 4 = character
}

/// Per-category tag indices from `selected_tags.csv` (rating, general,
/// character).
pub type CategoryIndices = (Vec<String>, Vec<usize>, Vec<usize>, Vec<usize>);

/// Parse `selected_tags.csv` exactly like the Python model wrapper: the
/// tag name is underscore-replaced unless it is a kaomoji; category
/// indices are recorded per category.
pub fn parse_tags_csv(csv_path: &Path) -> Result<CategoryIndices, WdTaggerError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(csv_path)
        .map_err(|source| WdTaggerError::Tags {
            path: csv_path.to_path_buf(),
            source: std::io::Error::other(source.to_string()),
        })?;
    let mut tags = Vec::new();
    let mut rating = Vec::new();
    let mut general = Vec::new();
    let mut character = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|source| WdTaggerError::Tags {
            path: csv_path.to_path_buf(),
            source: std::io::Error::other(source.to_string()),
        })?;
        let tag = record.get(0).unwrap_or_default().to_string();
        let tag = if KAOMOJIS.contains(&tag.as_str()) {
            tag
        } else {
            tag.replace('_', " ")
        };
        let category: i32 = record.get(1).unwrap_or_default().parse().unwrap_or(-1);
        match category {
            9 => rating.push(index),
            0 => general.push(index),
            4 => character.push(index),
            _ => {}
        }
        tags.push(tag);
    }
    Ok((tags, rating, general, character))
}

/// Preprocess an image into the model's NHWC BGR float input, port of
/// `WdTagger.get_model_inputs`: white-composite alpha, square-pad onto
/// white, bicubic resize to the model's input dimension, RGB -> BGR.
pub fn preprocess_image(
    image_path: &Path,
    input_dimension: u32,
) -> Result<Array4<f32>, WdTaggerError> {
    use image::imageops::FilterType;
    use image::RgbaImage;

    let decoded = image::open(image_path).map_err(|source| WdTaggerError::Image {
        path: image_path.to_path_buf(),
        source,
    })?;
    // White-composite the alpha channel.
    let mut canvas = RgbaImage::from_pixel(
        decoded.width(),
        decoded.height(),
        image::Rgba([255, 255, 255, 255]),
    );
    image::imageops::overlay(&mut canvas, &decoded.to_rgba8(), 0, 0);
    // Square-pad onto white (the composited canvas is already RGBA).
    let max_dimension = canvas.width().max(canvas.height());
    let mut padded = RgbaImage::from_pixel(
        max_dimension,
        max_dimension,
        image::Rgba([255, 255, 255, 255]),
    );
    let horizontal_padding = (max_dimension - canvas.width()) / 2;
    let vertical_padding = (max_dimension - canvas.height()) / 2;
    image::imageops::overlay(
        &mut padded,
        &canvas,
        horizontal_padding as i64,
        vertical_padding as i64,
    );
    // Resize (bicubic) to the model input.
    let resized = if max_dimension != input_dimension {
        image::imageops::resize(
            &padded,
            input_dimension,
            input_dimension,
            FilterType::CatmullRom,
        )
    } else {
        padded
    };
    // NHWC float32, RGB -> BGR.
    let mut array =
        Array4::<f32>::zeros((1, input_dimension as usize, input_dimension as usize, 3));
    for y in 0..input_dimension as usize {
        for x in 0..input_dimension as usize {
            let pixel = resized.get_pixel(x as u32, y as u32);
            array[[0, y, x, 0]] = pixel[2] as f32; // B
            array[[0, y, x, 1]] = pixel[1] as f32; // G
            array[[0, y, x, 2]] = pixel[0] as f32; // R
        }
    }
    Ok(array)
}

/// Settings mirroring the Python app's WD tagger panel.
#[derive(Debug, Clone)]
pub struct WdTaggerSettings {
    pub min_probability: f32,
    pub max_tags: usize,
    pub tags_to_exclude: Vec<String>,
}

impl Default for WdTaggerSettings {
    fn default() -> Self {
        Self {
            min_probability: 0.35,
            max_tags: 30,
            tags_to_exclude: Vec::new(),
        }
    }
}

/// A ready-to-run WD tagger: ONNX session + parsed tag list.
pub struct WdTagger {
    session: ort::session::Session,
    input_name: String,
    output_name: String,
    input_dimension: u32,
    pub tags: Vec<String>,
    pub rating_indices: Vec<usize>,
}

impl WdTagger {
    /// Load the ONNX model and its `selected_tags.csv` (sidecar in the
    /// same directory, like the Python `WdTaggerModel`).
    pub fn load(model_path: &Path) -> Result<Self, WdTaggerError> {
        let environment = ort::init()
            .build()
            .map_err(|source| WdTaggerError::Model(source.to_string()))?;
        let session = ort::session::Session::builder(&environment)
            .and_then(|mut builder| builder.commit_from_file(model_path))
            .map_err(|source| WdTaggerError::Model(source.to_string()))?;
        // Input shape: [batch?, H, W, 3]; dimension 1 is the side length.
        let input_dimension = session
            .inputs()
            .first()
            .and_then(|input| match input.dtype() {
                ort::value::ValueType::Tensor { shape, .. } => shape.to_ixdyn()[1].try_into().ok(),
                _ => None,
            })
            .unwrap_or(448) as u32;
        let input_name = session
            .inputs()
            .first()
            .map(|input| input.name().to_owned())
            .unwrap_or_default();
        let output_name = session
            .outputs()
            .first()
            .map(|output| output.name().to_owned())
            .unwrap_or_default();
        let tags_path = model_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("selected_tags.csv");
        let (tags, rating_indices, _general, _character) = parse_tags_csv(&tags_path)?;
        Ok(Self {
            session,
            input_name,
            output_name,
            input_dimension,
            tags,
            rating_indices,
        })
    }

    pub fn input_dimension(&self) -> u32 {
        self.input_dimension
    }

    /// Run inference and produce (tag, probability) pairs, port of
    /// `WdTaggerModel.generate_tags`: rating tags are excluded, tags
    /// below `min_probability` or in `tags_to_exclude` are dropped, the
    /// rest sorted by probability descending and truncated.
    pub fn generate_tags(
        &mut self,
        image_path: &Path,
        settings: &WdTaggerSettings,
    ) -> Result<Vec<(String, f32)>, WdTaggerError> {
        let input = preprocess_image(image_path, self.input_dimension)?;
        let array = ort::value::Tensor::from_array(input)
            .map_err(|source| WdTaggerError::Inference(source.to_string()))?;
        let outputs = self
            .session
            .run(ort::inputs![
                self.input_name.as_str() => array
            ])
            .map_err(|source| WdTaggerError::Inference(source.to_string()))?;
        let output_tensor = outputs
            .get(self.output_name.as_str())
            .ok_or_else(|| WdTaggerError::Inference("missing output".to_owned()))?;
        let probabilities = output_tensor
            .try_extract_array::<f32>()
            .map_err(|source| WdTaggerError::Inference(source.to_string()))?;
        let view = probabilities.view();
        // [1, n_tags] -> first row.
        let n_tags = self.tags.len();
        let mut tags_and_probabilities = Vec::new();
        for (index, tag) in self.tags.iter().enumerate() {
            if self.rating_indices.contains(&index) || index >= n_tags {
                continue;
            }
            let probability = view[[0, index]];
            if probability < settings.min_probability || settings.tags_to_exclude.contains(tag) {
                continue;
            }
            tags_and_probabilities.push((tag.clone(), probability));
        }
        tags_and_probabilities
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        tags_and_probabilities.truncate(settings.max_tags);
        Ok(tags_and_probabilities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parsing_underscore_replacement_and_categories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selected_tags.csv");
        std::fs::write(
            &path,
            "name,category,count\n1girl,0,9999\nlong_hair,0,8000\
             \n0_0,9,10\nhatsune_miku,4,500\nrating:general,9,9000\n",
        )
        .unwrap();
        let (tags, rating, general, character) = parse_tags_csv(&path).unwrap();
        assert_eq!(
            tags,
            [
                "1girl",
                "long hair",
                "0_0",
                "hatsune miku",
                "rating:general"
            ]
        );
        assert_eq!(rating, [2, 4]);
        assert_eq!(general, [0, 1]);
        assert_eq!(character, [3]);
    }

    #[test]
    fn preprocessing_produces_square_bgr_floats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.png");
        // 10x20 red image with a transparent left half -> white.
        let mut image = image::RgbaImage::new(10, 20);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 5 {
                image::Rgba([255, 0, 0, 128])
            } else {
                image::Rgba([255, 0, 0, 255])
            };
        }
        image.save(&path).unwrap();
        let array = preprocess_image(&path, 64).unwrap();
        assert_eq!(array.shape(), &[1, 64, 64, 3]);
        // Deep inside the opaque half (x >= 5 of the original maps past
        // the pad offset): pure red -> BGR ~[0, 0, 255].
        let opaque_r = array[[0, 32, 40, 2]];
        assert!((opaque_r - 255.0).abs() < 12.0, "opaque R {opaque_r}");
        let opaque_b = array[[0, 32, 40, 0]];
        assert!(opaque_b < 60.0, "opaque B {opaque_b}");
        // Corner pixel: white pad -> BGR [255, 255, 255].
        let corner = array[[0, 0, 0, 0]];
        assert!((corner - 255.0).abs() < 1.0);
    }

    #[test]
    fn tag_filtering_matches_python_semantics() {
        // Simulate generate_tags' filtering with a fake probability row:
        // ratings excluded, below-threshold dropped, sorted desc,
        // truncated.
        let tags: Vec<String> = vec![
            "a".into(),
            "b".into(),
            "rating:q".into(),
            "c".into(),
            "d".into(),
        ];
        let rating = vec![2usize];
        let probabilities = [0.9f32, 0.3, 0.99, 0.8, 0.7];
        let settings = WdTaggerSettings {
            min_probability: 0.35,
            max_tags: 2,
            tags_to_exclude: vec!["d".into()],
        };
        let mut kept: Vec<(String, f32)> = tags
            .iter()
            .enumerate()
            .filter(|(index, _)| !rating.contains(index))
            .map(|(index, tag)| (tag.clone(), probabilities[index]))
            .filter(|(tag, p)| {
                *p >= settings.min_probability && !settings.tags_to_exclude.contains(tag)
            })
            .collect();
        kept.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        kept.truncate(settings.max_tags);
        // Sorted descending, thresholded, rating excluded, 'd' excluded,
        // truncated to 2.
        assert_eq!(kept, vec![("a".to_string(), 0.9), ("c".to_string(), 0.8)]);
    }
}
