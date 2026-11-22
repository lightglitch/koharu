use koharu_ml::koharu_layout_highres::{KoharuLayoutHighres, default_thresholds};
use koharu_runtime::Device;
use serde::{Deserialize, Serialize};
use strum::{Display, VariantArray};

/// The detectors, by id in the section and by name.
#[derive(
    Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, Serialize, Deserialize, VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[derive(specta::Type)]
pub enum Detector {
    #[default]
    #[strum(to_string = "Koharu Layout HighRes")]
    KoharuLayoutHighres,
}

/// The lowest confidence each class keeps when `DetectorConfig` leaves it
/// unset, as the detector loads it.
// Fork addition, for Settings to show.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, specta::Type)]
pub struct DetectorThresholds {
    pub text: f32,
    pub onomatopoeia: f32,
    pub bubble: f32,
    pub panel: f32,
}

impl Detector {
    pub async fn thresholds(self) -> anyhow::Result<DetectorThresholds> {
        match self {
            Detector::KoharuLayoutHighres => {
                let found = default_thresholds().await?;
                Ok(DetectorThresholds {
                    text: found.text,
                    onomatopoeia: found.onomatopoeia,
                    bubble: found.bubble,
                    panel: found.panel,
                })
            }
        }
    }
}

pub(super) async fn load(
    model: Detector,
    device: &'static Device,
) -> anyhow::Result<KoharuLayoutHighres> {
    match model {
        Detector::KoharuLayoutHighres => KoharuLayoutHighres::load(device).await,
    }
}
