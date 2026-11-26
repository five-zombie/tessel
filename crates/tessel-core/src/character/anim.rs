use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::scene::NodeId;

use super::nano::{BoneId, Keyframe};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneChannel {
    pub bone_id: BoneId,
    pub keyframes: Vec<Keyframe>,
}

/// Single weight keyframe on a morph track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MorphWeightKey {
    pub time_secs: f32,
    pub weight: f32,
}

/// Weight animation for one named morph target on one mesh node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MorphWeightTrack {
    pub node: NodeId,
    pub target: String,
    pub keys: Vec<MorphWeightKey>,
}

/// Single step visibility keyframe for a scene node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VisibilityKey {
    pub time_secs: f32,
    pub visible: bool,
}

/// Step-interpolated visibility animation track for one scene node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibilityTrack {
    pub node: NodeId,
    pub keys: Vec<VisibilityKey>,
}

fn default_identity_quat() -> Quat {
    Quat::IDENTITY
}

/// Single dynamic parent-switch / prop-drop keyframe for a scene node.
/// When `bone_id` is `Some(bid)`, the node follows that bone's live transform (plus optional local offset).
/// When `bone_id` is `None`, the node detaches into world space at `(position, rotation)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParentSwitchKey {
    pub time_secs: f32,
    pub bone_id: Option<BoneId>,
    #[serde(default)]
    pub position: Vec3,
    #[serde(default = "default_identity_quat")]
    pub rotation: Quat,
}

/// Dynamic parent-switch / world-space prop drop animation track for one scene node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParentSwitchTrack {
    pub node: NodeId,
    pub keys: Vec<ParentSwitchKey>,
}

/// Per-node local TRS keyframe animation track (works without any skeletal bones,
/// and supports animating meshes, voxels, or `NodeKind::Group` hierarchies).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTransformTrack {
    pub node: NodeId,
    pub keyframes: Vec<Keyframe>,
}

/// Keyframe animation clip supporting linear translation/scale, SLERP rotation, morph weights,
/// visibility, parent-switch, and per-node transform tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub name: String,
    pub duration_secs: f32,
    pub channels: Vec<BoneChannel>,
    pub morph_tracks: Vec<MorphWeightTrack>,
    #[serde(default)]
    pub visibility_tracks: Vec<VisibilityTrack>,
    #[serde(default)]
    pub parent_switch_tracks: Vec<ParentSwitchTrack>,
    #[serde(default)]
    pub node_transform_tracks: Vec<NodeTransformTrack>,
    /// Optional quantized playback FPS (e.g. `Some(12.0)` for animated-on-twos pose stepping).
    #[serde(default)]
    pub stepped_fps: Option<f32>,
    /// Optional animation event/notify markers exported as glTF `extras` (`v5.7`).
    #[serde(default)]
    pub events: Vec<super::constraints::AnimationEventKey>,
}

impl AnimationClip {
    pub fn new_empty(name: impl Into<String>, duration_secs: f32) -> Self {
        Self {
            name: name.into(),
            duration_secs: duration_secs.max(0.1),
            channels: Vec::new(),
            morph_tracks: Vec::new(),
            visibility_tracks: Vec::new(),
            parent_switch_tracks: Vec::new(),
            node_transform_tracks: Vec::new(),
            stepped_fps: None,
            events: Vec::new(),
        }
    }

    /// Quantizes `time_secs` when `stepped_fps` is configured (enables 8/12 FPS stepped poses
    /// while the viewport camera continues rendering smoothly at 60 FPS).
    pub fn effective_time(&self, time_secs: f32) -> f32 {
        let t = time_secs.max(0.0);
        match self.stepped_fps {
            Some(fps) if fps.is_finite() && fps > 0.0 => (t * fps).floor() / fps,
            _ => t,
        }
    }

    /// Resamples all bone and node transform tracks at `fps` into `KeyframeInterpolation::Step`
    /// keyframes for 1:1 glTF 2.0 `STEP` stepped-animation export.
    pub fn bake_stepped(&self, fps: f32) -> Self {
        let safe_fps = if fps.is_finite() {
            fps.clamp(1.0, 60.0)
        } else {
            12.0
        };
        let duration = if self.duration_secs.is_finite() {
            self.duration_secs.clamp(0.05, 600.0)
        } else {
            1.0
        };
        let steps = ((duration * safe_fps).round() as usize).clamp(1, 2048);

        let mut src = self.clone();
        src.stepped_fps = None;

        let mut baked = Self::new_empty(self.name.clone(), duration);
        baked.stepped_fps = Some(safe_fps);
        baked.visibility_tracks = self.visibility_tracks.clone();
        baked.parent_switch_tracks = self.parent_switch_tracks.clone();

        for step in 0..=steps {
            let t = if step == steps {
                duration
            } else {
                (step as f32 / steps as f32) * duration
            };
            for ch in &src.channels {
                if let Some(mut kf) = src.sample_bone(ch.bone_id, t) {
                    kf.time_secs = t;
                    kf.interpolation = super::nano::KeyframeInterpolation::Step;
                    baked.upsert_keyframe(ch.bone_id, kf);
                }
            }
            for (node, mut kf) in src.sample_node_transforms(t) {
                kf.time_secs = t;
                kf.interpolation = super::nano::KeyframeInterpolation::Step;
                baked.upsert_node_transform_key(node, kf);
            }
            for (node, target, weight) in src.sample_morph_weights(t) {
                baked.upsert_morph_weight(node, &target, t, weight);
            }
        }
        baked
    }

    /// Starter 19-bone humanoid Idle animation (subtle breathing + head/arm relaxation).
    pub fn humanoid_idle() -> Self {
        let mut clip = Self::new_empty("Idle", 2.0);
        // Spine (1) gentle breathing pitch
        clip.upsert_keyframe(BoneId(1), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(1),
            Keyframe::rotation_only(1.0, Quat::from_rotation_x(0.03)),
        );
        clip.upsert_keyframe(BoneId(1), Keyframe::rotation_only(2.0, Quat::IDENTITY));
        // Head (4) subtle look sway
        clip.upsert_keyframe(BoneId(4), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(4),
            Keyframe::rotation_only(1.0, Quat::from_rotation_y(0.05)),
        );
        clip.upsert_keyframe(BoneId(4), Keyframe::rotation_only(2.0, Quat::IDENTITY));
        // Arms (6 = LeftUpperArm, 10 = RightUpperArm) subtle breath lift
        clip.upsert_keyframe(BoneId(6), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(6),
            Keyframe::rotation_only(1.0, Quat::from_rotation_z(0.04)),
        );
        clip.upsert_keyframe(BoneId(6), Keyframe::rotation_only(2.0, Quat::IDENTITY));
        clip.upsert_keyframe(BoneId(10), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(10),
            Keyframe::rotation_only(1.0, Quat::from_rotation_z(-0.04)),
        );
        clip.upsert_keyframe(BoneId(10), Keyframe::rotation_only(2.0, Quat::IDENTITY));
        clip
    }

    /// Starter 19-bone humanoid Walk cycle (counter-swinging arms, legs, knees, elbows).
    pub fn humanoid_walk() -> Self {
        let mut clip = Self::new_empty("Walk", 1.0);
        let swing = 0.42f32;
        let knee = 0.48f32;
        let elbow = -0.35f32;
        // LeftThigh (13) & RightThigh (16)
        for &(bone, sign) in &[(13u32, 1.0f32), (16u32, -1.0f32)] {
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(0.0, Quat::IDENTITY));
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(0.25, Quat::from_rotation_x(-swing * sign)),
            );
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(0.5, Quat::IDENTITY));
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(0.75, Quat::from_rotation_x(swing * sign)),
            );
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(1.0, Quat::IDENTITY));
        }
        // LeftCalf (14) & RightCalf (17) knee bend on swing phase
        clip.upsert_keyframe(BoneId(14), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(14),
            Keyframe::rotation_only(0.25, Quat::from_rotation_x(knee)),
        );
        clip.upsert_keyframe(BoneId(14), Keyframe::rotation_only(0.5, Quat::IDENTITY));
        clip.upsert_keyframe(BoneId(14), Keyframe::rotation_only(1.0, Quat::IDENTITY));
        clip.upsert_keyframe(BoneId(17), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(BoneId(17), Keyframe::rotation_only(0.5, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(17),
            Keyframe::rotation_only(0.75, Quat::from_rotation_x(knee)),
        );
        clip.upsert_keyframe(BoneId(17), Keyframe::rotation_only(1.0, Quat::IDENTITY));
        // LeftUpperArm (6) & RightUpperArm (10) counter-swing
        for &(bone, sign) in &[(6u32, -1.0f32), (10u32, 1.0f32)] {
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(0.0, Quat::IDENTITY));
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(0.25, Quat::from_rotation_x(-swing * 0.7 * sign)),
            );
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(0.5, Quat::IDENTITY));
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(0.75, Quat::from_rotation_x(swing * 0.7 * sign)),
            );
            clip.upsert_keyframe(BoneId(bone), Keyframe::rotation_only(1.0, Quat::IDENTITY));
        }
        // Forearms (7, 11) slight elbow flex
        for bone in [7u32, 11u32] {
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(0.0, Quat::from_rotation_x(elbow)),
            );
            clip.upsert_keyframe(
                BoneId(bone),
                Keyframe::rotation_only(1.0, Quat::from_rotation_x(elbow)),
            );
        }
        clip
    }

    /// Inserts or replaces a keyframe (within 1ms tolerance) for `bone_id`, keeping timestamps sorted.
    pub fn upsert_keyframe(&mut self, bone_id: BoneId, keyframe: Keyframe) {
        let time = keyframe.time_secs.max(0.0);
        let kf = Keyframe {
            time_secs: time,
            ..keyframe
        };
        if time > self.duration_secs {
            self.duration_secs = time;
        }
        if let Some(channel) = self.channels.iter_mut().find(|c| c.bone_id == bone_id) {
            if let Some(existing) = channel
                .keyframes
                .iter_mut()
                .find(|k| (k.time_secs - time).abs() <= 1e-3)
            {
                *existing = kf;
            } else {
                channel.keyframes.push(kf);
                channel
                    .keyframes
                    .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            }
        } else {
            self.channels.push(BoneChannel {
                bone_id,
                keyframes: vec![kf],
            });
        }
    }

    /// Removes the keyframe matching `time_secs` (within 1ms tolerance) for `bone_id`.
    pub fn remove_keyframe(&mut self, bone_id: BoneId, time_secs: f32) -> Option<Keyframe> {
        let ch_idx = self.channels.iter().position(|c| c.bone_id == bone_id)?;
        let kf_idx = self.channels[ch_idx]
            .keyframes
            .iter()
            .position(|k| (k.time_secs - time_secs).abs() <= 1e-3)?;
        let removed = self.channels[ch_idx].keyframes.remove(kf_idx);
        if self.channels[ch_idx].keyframes.is_empty() {
            self.channels.remove(ch_idx);
        }
        Some(removed)
    }

    /// Inserts or replaces a morph weight keyframe (within 1ms tolerance),
    /// keeping timestamps sorted.
    pub fn upsert_morph_weight(&mut self, node: NodeId, target: &str, time_secs: f32, weight: f32) {
        let time = time_secs.max(0.0);
        if time > self.duration_secs {
            self.duration_secs = time;
        }
        if let Some(track) = self
            .morph_tracks
            .iter_mut()
            .find(|t| t.node == node && t.target == target)
        {
            if let Some(existing) = track
                .keys
                .iter_mut()
                .find(|k| (k.time_secs - time).abs() <= 1e-3)
            {
                existing.weight = weight;
            } else {
                track.keys.push(MorphWeightKey {
                    time_secs: time,
                    weight,
                });
                track
                    .keys
                    .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            }
        } else {
            self.morph_tracks.push(MorphWeightTrack {
                node,
                target: target.to_string(),
                keys: vec![MorphWeightKey {
                    time_secs: time,
                    weight,
                }],
            });
        }
    }

    /// Removes the morph weight keyframe matching `time_secs` (within 1ms
    /// tolerance); drops the track when its last key is removed.
    pub fn remove_morph_weight(
        &mut self,
        node: NodeId,
        target: &str,
        time_secs: f32,
    ) -> Option<f32> {
        let t_idx = self
            .morph_tracks
            .iter()
            .position(|t| t.node == node && t.target == target)?;
        let k_idx = self.morph_tracks[t_idx]
            .keys
            .iter()
            .position(|k| (k.time_secs - time_secs).abs() <= 1e-3)?;
        let removed = self.morph_tracks[t_idx].keys.remove(k_idx);
        if self.morph_tracks[t_idx].keys.is_empty() {
            self.morph_tracks.remove(t_idx);
        }
        Some(removed.weight)
    }

    /// Samples every morph track at `time_secs` (linear interp, clamped ends).
    pub fn sample_morph_weights(&self, time_secs: f32) -> Vec<(NodeId, String, f32)> {
        let time_secs = self.effective_time(time_secs);
        let mut out = Vec::with_capacity(self.morph_tracks.len());
        for track in &self.morph_tracks {
            if track.keys.is_empty() {
                continue;
            }
            let w = if track.keys.len() == 1 || time_secs <= track.keys[0].time_secs {
                track.keys[0].weight
            } else if time_secs >= track.keys.last().map(|k| k.time_secs).unwrap_or(0.0) {
                track.keys.last().map(|k| k.weight).unwrap_or(0.0)
            } else {
                let mut w = track.keys.last().map(|k| k.weight).unwrap_or(0.0);
                for pair in track.keys.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if time_secs >= a.time_secs && time_secs < b.time_secs {
                        let span = (b.time_secs - a.time_secs).max(1e-5);
                        let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
                        w = a.weight + (b.weight - a.weight) * alpha;
                        break;
                    }
                }
                w
            };
            out.push((track.node, track.target.clone(), w));
        }
        out
    }

    /// Inserts or replaces a step visibility keyframe (within 1ms tolerance) for `node`.
    pub fn upsert_visibility_key(&mut self, node: NodeId, time_secs: f32, visible: bool) {
        let time = time_secs.max(0.0);
        if time > self.duration_secs {
            self.duration_secs = time;
        }
        if let Some(track) = self.visibility_tracks.iter_mut().find(|t| t.node == node) {
            if let Some(existing) = track
                .keys
                .iter_mut()
                .find(|k| (k.time_secs - time).abs() <= 1e-3)
            {
                existing.visible = visible;
            } else {
                track.keys.push(VisibilityKey {
                    time_secs: time,
                    visible,
                });
                track
                    .keys
                    .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            }
        } else {
            self.visibility_tracks.push(VisibilityTrack {
                node,
                keys: vec![VisibilityKey {
                    time_secs: time,
                    visible,
                }],
            });
        }
    }

    /// Samples every node visibility track at `time_secs` (step hold from latest key <= time_secs).
    pub fn sample_node_visibility(&self, time_secs: f32) -> Vec<(NodeId, bool)> {
        let time_secs = self.effective_time(time_secs);
        let mut out = Vec::with_capacity(self.visibility_tracks.len());
        for track in &self.visibility_tracks {
            let Some(first) = track.keys.first() else {
                continue;
            };
            let mut val = first.visible;
            for k in &track.keys {
                if k.time_secs <= time_secs + 1e-4 {
                    val = k.visible;
                } else {
                    break;
                }
            }
            out.push((track.node, val));
        }
        out
    }

    /// Inserts or replaces a dynamic parent-switch / prop-drop keyframe (within 1ms tolerance) for `node`.
    pub fn upsert_parent_switch_key(&mut self, node: NodeId, key: ParentSwitchKey) {
        let time = key.time_secs.max(0.0);
        let k = ParentSwitchKey {
            time_secs: time,
            ..key
        };
        if time > self.duration_secs {
            self.duration_secs = time;
        }
        if let Some(track) = self
            .parent_switch_tracks
            .iter_mut()
            .find(|t| t.node == node)
        {
            if let Some(existing) = track
                .keys
                .iter_mut()
                .find(|x| (x.time_secs - time).abs() <= 1e-3)
            {
                *existing = k;
            } else {
                track.keys.push(k);
                track
                    .keys
                    .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            }
        } else {
            self.parent_switch_tracks.push(ParentSwitchTrack {
                node,
                keys: vec![k],
            });
        }
    }

    /// Samples every parent-switch track at `time_secs`.
    /// Interpolates `(position, rotation)` smoothly between consecutive keys sharing the same `bone_id`
    /// (including two detached `None` world-space keys for weapon drop / grenade throw arcs),
    /// and step-switches when changing parent bones.
    pub fn sample_parent_switches(&self, time_secs: f32) -> Vec<(NodeId, ParentSwitchKey)> {
        let time_secs = self.effective_time(time_secs);
        let mut out = Vec::with_capacity(self.parent_switch_tracks.len());
        for track in &self.parent_switch_tracks {
            let Some(first) = track.keys.first().copied() else {
                continue;
            };
            if track.keys.len() == 1 || time_secs <= first.time_secs {
                out.push((track.node, first));
                continue;
            }
            let last = track.keys.last().copied().unwrap_or(first);
            if time_secs >= last.time_secs {
                out.push((track.node, last));
                continue;
            }
            let mut sampled = last;
            for pair in track.keys.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if time_secs >= a.time_secs && time_secs < b.time_secs {
                    if a.bone_id == b.bone_id {
                        let span = (b.time_secs - a.time_secs).max(1e-5);
                        let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
                        sampled = ParentSwitchKey {
                            time_secs,
                            bone_id: a.bone_id,
                            position: a.position.lerp(b.position, alpha),
                            rotation: a.rotation.slerp(b.rotation, alpha),
                        };
                    } else {
                        sampled = a;
                    }
                    break;
                }
            }
            out.push((track.node, sampled));
        }
        out
    }

    /// Inserts or replaces a per-node local TRS delta keyframe (within 1ms tolerance),
    /// keeping timestamps sorted and extending `duration_secs` if needed.
    pub fn upsert_node_transform_key(&mut self, node: NodeId, keyframe: Keyframe) {
        let time = keyframe.time_secs.max(0.0);
        if time > self.duration_secs {
            self.duration_secs = time;
        }
        let kf = Keyframe {
            time_secs: time,
            ..keyframe
        };
        if let Some(track) = self
            .node_transform_tracks
            .iter_mut()
            .find(|t| t.node == node)
        {
            if let Some(existing) = track
                .keyframes
                .iter_mut()
                .find(|k| (k.time_secs - time).abs() <= 1e-3)
            {
                *existing = kf;
            } else {
                track.keyframes.push(kf);
                track
                    .keyframes
                    .sort_by(|a, b| a.time_secs.total_cmp(&b.time_secs));
            }
        } else {
            self.node_transform_tracks.push(NodeTransformTrack {
                node,
                keyframes: vec![kf],
            });
        }
    }

    /// Samples every per-node transform track at `time_secs`.
    pub fn sample_node_transforms(&self, time_secs: f32) -> Vec<(NodeId, Keyframe)> {
        let time_secs = self.effective_time(time_secs);
        let mut out = Vec::with_capacity(self.node_transform_tracks.len());
        for track in &self.node_transform_tracks {
            let Some(&first) = track.keyframes.first() else {
                continue;
            };
            if track.keyframes.len() == 1 || time_secs <= first.time_secs {
                out.push((track.node, first));
                continue;
            }
            let last = track.keyframes.last().copied().unwrap_or(first);
            if time_secs >= last.time_secs {
                out.push((track.node, last));
                continue;
            }
            let mut sampled = last;
            for pair in track.keyframes.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if time_secs >= a.time_secs && time_secs < b.time_secs {
                    let span = (b.time_secs - a.time_secs).max(1e-5);
                    let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
                    let eased = a.interpolation.apply(alpha);
                    sampled = Keyframe {
                        time_secs,
                        translation: a.translation.lerp(b.translation, eased),
                        rotation: a.rotation.slerp(b.rotation, eased),
                        scale: a.scale.lerp(b.scale, eased),
                        interpolation: a.interpolation,
                    };
                    break;
                }
            }
            out.push((track.node, sampled));
        }
        out
    }

    /// Returns deduplicated sorted keyframe timestamps for a specific bone or the whole clip.
    pub fn keyframe_times(&self, bone_id: Option<BoneId>) -> Vec<f32> {
        let mut times: Vec<f32> = self
            .channels
            .iter()
            .filter(|c| bone_id.is_none_or(|bid| c.bone_id == bid))
            .flat_map(|c| c.keyframes.iter().map(|k| k.time_secs))
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() <= 1e-3);
        times
    }

    pub fn sample_bone(&self, bone_id: BoneId, time_secs: f32) -> Option<Keyframe> {
        let time_secs = self.effective_time(time_secs);
        let channel = self.channels.iter().find(|c| c.bone_id == bone_id)?;
        if channel.keyframes.is_empty() {
            return None;
        }
        if channel.keyframes.len() == 1 || time_secs <= channel.keyframes[0].time_secs {
            return Some(channel.keyframes[0]);
        }
        let last = *channel.keyframes.last()?;
        if time_secs >= last.time_secs {
            return Some(last);
        }

        for pair in channel.keyframes.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if time_secs >= a.time_secs && time_secs <= b.time_secs {
                let span = (b.time_secs - a.time_secs).max(1e-5);
                let alpha = ((time_secs - a.time_secs) / span).clamp(0.0, 1.0);
                let eased = a.interpolation.apply(alpha);
                return Some(Keyframe {
                    time_secs,
                    translation: a.translation.lerp(b.translation, eased),
                    rotation: a.rotation.slerp(b.rotation, eased),
                    scale: a.scale.lerp(b.scale, eased),
                    interpolation: a.interpolation,
                });
            }
        }
        Some(last)
    }

    pub fn starter_idle() -> Self {
        Self {
            name: "Idle".into(),
            duration_secs: 2.0,
            morph_tracks: Vec::new(),
            visibility_tracks: Vec::new(),
            parent_switch_tracks: Vec::new(),
            node_transform_tracks: Vec::new(),
            stepped_fps: None,
            events: Vec::new(),
            channels: vec![BoneChannel {
                bone_id: BoneId(1), // Torso subtle breathing
                keyframes: vec![
                    Keyframe {
                        time_secs: 0.0,
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: Default::default(),
                    },
                    Keyframe {
                        time_secs: 1.0,
                        translation: Vec3::new(0.0, 0.03, 0.0),
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: Default::default(),
                    },
                    Keyframe {
                        time_secs: 2.0,
                        translation: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        scale: Vec3::ONE,
                        interpolation: Default::default(),
                    },
                ],
            }],
        }
    }

    pub fn starter_walk() -> Self {
        let fwd = Quat::from_rotation_x(0.45);
        let back = Quat::from_rotation_x(-0.45);
        Self {
            name: "Walk".into(),
            duration_secs: 1.0,
            morph_tracks: Vec::new(),
            visibility_tracks: Vec::new(),
            parent_switch_tracks: Vec::new(),
            node_transform_tracks: Vec::new(),
            stepped_fps: None,
            events: Vec::new(),
            channels: vec![
                BoneChannel {
                    bone_id: BoneId(3), // LeftArm
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, fwd),
                        Keyframe::rotation_only(0.5, back),
                        Keyframe::rotation_only(1.0, fwd),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(4), // RightArm
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, back),
                        Keyframe::rotation_only(0.5, fwd),
                        Keyframe::rotation_only(1.0, back),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(5), // LeftLeg
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, back),
                        Keyframe::rotation_only(0.5, fwd),
                        Keyframe::rotation_only(1.0, back),
                    ],
                },
                BoneChannel {
                    bone_id: BoneId(6), // RightLeg
                    keyframes: vec![
                        Keyframe::rotation_only(0.0, fwd),
                        Keyframe::rotation_only(0.5, back),
                        Keyframe::rotation_only(1.0, fwd),
                    ],
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::CharacterRig;

    #[test]
    fn walk_animation_interpolates_smoothly_and_updates_world_matrices() {
        let mut rig = CharacterRig::biped_template();
        rig.active_animation = 1; // Walk
        rig.playback_time = 0.25;
        rig.apply_active_animation();
        let mats = rig.world_pose_matrices();
        assert_eq!(mats.len(), 7);
        assert!(mats[&BoneId(5)].is_finite());
    }

    #[test]
    fn upsert_and_remove_keyframes_maintain_sorted_order() {
        let mut clip = AnimationClip::new_empty("Wave", 1.0);
        clip.upsert_keyframe(
            BoneId(4),
            Keyframe::rotation_only(1.5, Quat::from_rotation_z(0.8)),
        );
        clip.upsert_keyframe(BoneId(4), Keyframe::rotation_only(0.0, Quat::IDENTITY));
        clip.upsert_keyframe(
            BoneId(4),
            Keyframe::rotation_only(0.5, Quat::from_rotation_z(1.2)),
        );
        // Overwrite existing 0.5s keyframe within tolerance
        clip.upsert_keyframe(
            BoneId(4),
            Keyframe::rotation_only(0.5002, Quat::from_rotation_z(1.0)),
        );

        assert!((clip.duration_secs - 1.5).abs() < 1e-5);
        assert_eq!(clip.keyframe_times(Some(BoneId(4))).len(), 3);
        assert!(clip.remove_keyframe(BoneId(4), 0.5).is_some());
        assert_eq!(clip.keyframe_times(Some(BoneId(4))).len(), 2);
    }

    #[test]
    fn morph_weight_tracks_upsert_sample_and_remove() {
        use crate::scene::NodeId;

        let node = NodeId(7);
        let mut clip = AnimationClip::new_empty("Blink", 2.0);
        clip.upsert_morph_weight(node, "Smile", 0.0, 0.0);
        clip.upsert_morph_weight(node, "Smile", 1.0, 1.0);
        clip.upsert_morph_weight(node, "Smile", 0.5, 0.5);
        assert_eq!(clip.morph_tracks.len(), 1);
        assert_eq!(clip.morph_tracks[0].keys.len(), 3);

        // Linear interpolation between keys, clamped ends.
        let at = |t: f32| {
            clip.sample_morph_weights(t)
                .into_iter()
                .find(|(n, name, _)| *n == node && name == "Smile")
                .map(|(_, _, w)| w)
                .unwrap()
        };
        assert!((at(0.0) - 0.0).abs() < 1e-6);
        assert!((at(0.25) - 0.25).abs() < 1e-6);
        assert!((at(2.0) - 1.0).abs() < 1e-6);

        // Overwrite within tolerance, then remove.
        clip.upsert_morph_weight(node, "Smile", 0.5002, 0.9);
        assert_eq!(clip.morph_tracks[0].keys.len(), 3);
        assert_eq!(clip.remove_morph_weight(node, "Smile", 1.0), Some(1.0));
        assert_eq!(clip.remove_morph_weight(node, "Smile", 9.9), None);
        assert_eq!(clip.morph_tracks[0].keys.len(), 2);
        assert_eq!(clip.remove_morph_weight(node, "Smile", 0.0), Some(0.0));
        assert_eq!(clip.remove_morph_weight(node, "Smile", 0.5), Some(0.9));
        assert!(clip.morph_tracks.is_empty());
    }

    #[test]
    fn cubic_bezier_and_stepped_fps_quantize_and_bake() {
        use crate::character::KeyframeInterpolation;

        let mut clip = AnimationClip::new_empty("InkSlash", 1.0);
        clip.stepped_fps = Some(8.0);
        clip.upsert_keyframe(
            BoneId(1),
            Keyframe {
                time_secs: 0.0,
                translation: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                interpolation: KeyframeInterpolation::CubicBezier {
                    p1x: 0.25,
                    p1y: 0.1,
                    p2x: 0.25,
                    p2y: 1.0,
                },
            },
        );
        clip.upsert_keyframe(
            BoneId(1),
            Keyframe {
                time_secs: 1.0,
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                interpolation: KeyframeInterpolation::Linear,
            },
        );
        // Quantized at 8 FPS (0.125s steps): t=0.05 and t=0.10 both snap to 0.0s.
        let s0 = clip.sample_bone(BoneId(1), 0.05).unwrap();
        let s1 = clip.sample_bone(BoneId(1), 0.10).unwrap();
        assert!((s0.translation.y - s1.translation.y).abs() < 1e-6);
        // At t=0.25s (step 2), ease-out cubic bezier should be well ahead of linear 0.25.
        let s2 = clip.sample_bone(BoneId(1), 0.25).unwrap();
        assert!(s2.translation.y > 0.35);

        let baked = clip.bake_stepped(8.0);
        assert_eq!(baked.channels.len(), 1);
        assert!(baked.channels[0].keyframes.len() >= 8);
        assert!(baked.channels[0]
            .keyframes
            .iter()
            .all(|k| k.interpolation == KeyframeInterpolation::Step));
    }
}
