//! Memory-lifecycle signal mixer.

use raki_domain::{MixerConfig, NoteSignals, SignalBooster, SignalBreakdown};

pub struct DefaultSignalBooster {
    config: MixerConfig,
}

impl DefaultSignalBooster {
    pub fn new(config: MixerConfig) -> Self {
        Self { config }
    }
}

impl SignalBooster for DefaultSignalBooster {
    fn boost(
        &self,
        retrieval_score: f64,
        signals: &NoteSignals,
        now_ms: i64,
    ) -> (f64, SignalBreakdown) {
        let cfg = self.config;

        let elapsed_days = signals.last_accessed_at_ms.map(|t| {
            let ms = (now_ms - t).max(0) as f64;
            ms / 86_400_000.0
        });
        let recency = elapsed_days
            .map(|d| 2.0_f64.powf(-d / cfg.half_life_days))
            .unwrap_or(0.0);

        let pin_value = if signals.pinned { 1.0 } else { 0.0 };
        let pin = cfg.pin_boost * pin_value;

        let salience_norm =
            ((1.0 + signals.view_count as f64).ln() / 10.0_f64.ln()).clamp(0.0, 1.0);
        let salience = cfg.salience_weight * salience_norm;

        let raw = 1.0 + recency + pin + salience;
        let capped = raw.min(cfg.max_boost);

        let breakdown = SignalBreakdown {
            recency,
            pin,
            salience,
            raw_boost: raw,
            capped_boost: capped,
        };

        (retrieval_score * capped, breakdown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{MixerConfig, NoteSignals};

    fn booster() -> DefaultSignalBooster {
        DefaultSignalBooster::new(MixerConfig::new(7.0, 0.25, 0.15, 2.0).unwrap())
    }

    #[test]
    fn pinned_note_gets_boost() {
        let b = booster();
        let signals = NoteSignals {
            pinned: true,
            ..Default::default()
        };
        let (score, breakdown) = b.boost(1.0, &signals, 0);
        assert!(score > 1.0);
        assert_eq!(breakdown.pin, 0.25);
        assert_eq!(breakdown.capped_boost, 1.25);
    }

    #[test]
    fn recent_note_gets_higher_boost_than_old() {
        let b = booster();
        let now = 1_000_000_000_000i64;
        let recent = NoteSignals {
            last_accessed_at_ms: Some(now),
            ..Default::default()
        };
        let old = NoteSignals {
            last_accessed_at_ms: Some(now - 30 * 86_400_000),
            ..Default::default()
        };
        let (recent_score, _) = b.boost(1.0, &recent, now);
        let (old_score, _) = b.boost(1.0, &old, now);
        assert!(recent_score > old_score);
    }

    #[test]
    fn max_boost_caps_extreme_signals() {
        let b = booster();
        let now = 1_000_000_000_000i64;
        let signals = NoteSignals {
            pinned: true,
            view_count: 1000,
            last_accessed_at_ms: Some(now),
        };
        let (score, breakdown) = b.boost(1.0, &signals, now);
        assert_eq!(score, 2.0);
        assert_eq!(breakdown.capped_boost, 2.0);
    }
}
