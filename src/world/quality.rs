//! How much of a scene's costly detail a device draws: the whole of it (`High`, the default), or
//! less glow, ground cover and particles for a slower phone. The design of a scene is kept; only
//! how dense and how bright its extras are is cut, so a game can offer it as a setting.

use serde::{Deserialize, Serialize};

/// A level of detail for every scene drawn (`WorldRenderer::set_quality`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    #[default]
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Medium, Quality::High];

    pub fn name(self) -> &'static str {
        match self { Quality::Low => "Low", Quality::Medium => "Medium", Quality::High => "High" }
    }

    /// The level named `name` (any case), if it is one.
    pub fn parse(name: &str) -> Option<Quality> {
        Quality::ALL.into_iter().find(|q| q.name().eq_ignore_ascii_case(name.trim()))
    }

    /// The share of a scene's bloom kept: bloom's blur passes are most of post on a phone.
    pub fn bloom(self) -> f32 {
        match self { Quality::Low => 0.0, Quality::Medium => 0.6, Quality::High => 1.0 }
    }

    /// The share of ground-cover tufts kept (grass, flowers, moss).
    pub fn cover(self) -> f32 {
        match self { Quality::Low => 0.45, Quality::Medium => 0.75, Quality::High => 1.0 }
    }

    /// The share of drifting particles kept (leaves, motes, embers).
    pub fn particles(self) -> f32 {
        match self { Quality::Low => 0.4, Quality::Medium => 0.7, Quality::High => 1.0 }
    }

    /// `count` particles cut to this level's share, never all of a layer that has some.
    pub fn particle_count(self, count: u32) -> u32 {
        if count == 0 { return 0; }
        ((count as f32 * self.particles()).round() as u32).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::Quality;

    #[test]
    fn each_level_reads_back_and_lower_levels_draw_less() {
        for q in Quality::ALL { assert_eq!(Quality::parse(&q.name().to_lowercase()), Some(q)); }
        assert_eq!(Quality::default(), Quality::High);
        assert!(Quality::Low.cover() < Quality::Medium.cover() && Quality::Medium.cover() < Quality::High.cover());
        assert_eq!(Quality::High.particle_count(40), 40);
        assert_eq!(Quality::Low.particle_count(1), 1);
        assert_eq!(Quality::Low.particle_count(0), 0);
    }
}
