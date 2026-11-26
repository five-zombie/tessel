use serde_json::{json, Value};

pub(super) fn character_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "character_create_clip",
            "description": "Create a new empty animation clip (auto-initializes an empty animation container if no character rig exists yet).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.05, "maximum": 600.0 },
                    "activate": { "type": "boolean" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "character_update_clip",
            "description": "Rename an existing animation clip and/or update its duration_secs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.05, "maximum": 600.0 }
                }
            }
        }),
        json!({
            "name": "character_apply_biped_rig",
            "description": "Apply a humanoid skeleton rig (Right-Handed +Y Up, +Z Front: Left=+X, Right=-X). Pass preset='starter' (default, 7 bones) or preset='humanoid' (19-bone 1:1 humanoid). For multi-character scenes, pass append=true with prefix (e.g. 'P2_'), offset [x,y,z], and yaw_deg (e.g. 180) to spawn an additional prefixed skeleton into the active rig in 1 call and receive its bone_ids map.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["starter", "humanoid"] },
                    "append": { "type": "boolean", "description": "When true, append a new sub-skeleton into the existing CharacterRig instead of replacing it (ideal for 2+ character scenes)" },
                    "prefix": { "type": "string", "description": "Bone name prefix when append=true (e.g. 'P2_')" },
                    "offset": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space root translation [x, y, z] for appended sub-skeleton" },
                    "yaw_deg": { "type": "number", "description": "Yaw rotation in degrees around +Y applied to the appended sub-skeleton's Root rest orientation (so child rotations remain character-local)" },
                    "include_default_clips": { "type": "boolean", "description": "Include default Idle & Walk animation clips (default true). Pass false to start with 0 default clips (or skip remapping Idle & Walk when append=true)." }
                }
            }
        }),
        json!({
            "name": "character_set_bone_rest_position",
            "description": "Move an existing bone's local rest position [x, y, z] and/or rest rotation_deg [pitch, yaw, roll] without deleting its subtree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id"]
            }
        }),
        json!({
            "name": "character_add_bone",
            "description": "Add a custom bone to the active rig (or initialize a custom prop rig for backpacks, chests, weapons) with optional parent_bone_id and rest position [x,y,z]. By default (space='local'), 'position' is parent-relative when parent_bone_id is set (and world space when root). Pass space='world' to supply world coordinates for child bones.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "parent_bone_id": { "type": ["integer", "null"], "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Parent-relative local rest offset [x,y,z] when parent_bone_id is set (or world space if root or space='world')" },
                    "space": { "type": "string", "enum": ["local", "world"], "description": "Coordinate space of 'position': 'local' (default, parent-relative) or 'world' (converted automatically to parent-relative offset)" }
                },
                "required": ["name", "position"]
            }
        }),
        json!({
            "name": "character_bind_node",
            "description": "Bind a scene node (e.g. backpack flap, pouch, weapon part) rigidly to a bone_id (or null to unbind for smooth auto-weight skinning).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "bone_id": { "type": ["integer", "null"], "minimum": 0 }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "character_set_node_bone_mask",
            "description": "Restrict which bones may auto-skin a scene node (e.g. allow only leg/root bones on trousers so hanging arms never pull the hips). Pass null or empty array to clear the mask.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "bone_ids": { "type": ["array", "null"], "items": { "type": "integer", "minimum": 0 } }
                },
                "required": ["node_id"]
            }
        }),
        json!({
            "name": "character_set_bone_pose",
            "description": "Set a bone's live pose rotation_deg [pitch, yaw, roll] and/or translation [dx, dy, dz] offset from rest pose on the active character rig (captured by character_snapshot_pose_to_keyframe). Right-Handed (+Y Up, +Z Front, +X Left): positive pitch (+X) tilts/falls FORWARD, negative pitch (-X) tilts/falls BACKWARD in character-local space. Pass dry_run=true to preview resulting orientation vectors and world position without mutating the rig.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Local Euler degrees [pitch_x, yaw_y, roll_z] (Right-Handed +Y Up/+Z Front/+X Left: +X pitch tilts forward, +Y yaw turns toward +X/Left)" },
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Optional local translation offset [dx, dy, dz] relative to rest pose" },
                    "dry_run": { "type": "boolean", "description": "When true, returns resulting local/world orientation vectors and world position without mutating the scene or undo history (default false)" }
                },
                "required": ["bone_id"]
            }
        }),
        json!({
            "name": "character_set_clip_layer",
            "description": "Assign an independent animation clip to a bone prefix (e.g. P1 '' plays Walk while P2 'P2_' plays ShotFall simultaneously). Pass clear=true to remove all clip layers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "bone_prefix": { "type": "string", "description": "Bone name prefix for this layer (e.g. 'P2_', or '' for base layer)" },
                    "time_offset": { "type": "number", "description": "Optional playback time offset in seconds" },
                    "clear": { "type": "boolean", "description": "When true, clears all clip layers" }
                }
            }
        }),
        json!({
            "name": "character_bake_clip_layers",
            "description": "Bake the current multi-character clip_layers composition (including rotations, translations, morph weights, and visibility tracks) into a single permanent AnimationClip ready for .glb export and viewport_render_turntable_gif. Optionally clamps all characters to floor_y at every sampled frame.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the baked clip (replaces existing clip with same name)" },
                    "fps": { "type": "number", "minimum": 1, "maximum": 120, "description": "Sampling frame rate (default 24)" },
                    "duration_secs": { "type": "number", "minimum": 0.05, "description": "Optional duration override in seconds (defaults to max duration across clip layers)" },
                    "floor_y": { "type": "number", "description": "Optional floor Y height to clamp every character onto at each baked sample" },
                    "activate": { "type": "boolean", "description": "Make the baked clip active (default true)" },
                    "clear_layers": { "type": "boolean", "description": "Clear clip_layers after baking (default false)" }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "character_solve_chain_ik",
            "description": "Solve multi-bone chain IK (CCD) for any parent-child chain of 2..=32 bones (tails, spines, tentacles, limbs) so the end bone reaches target [x,y,z].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_ids": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 0 },
                        "minItems": 2,
                        "maxItems": 32
                    },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "max_iterations": { "type": "integer", "minimum": 1, "maximum": 64 },
                    "tolerance": { "type": "number", "minimum": 0.0001, "maximum": 1.0 }
                },
                "required": ["bone_ids", "target"]
            }
        }),
        json!({
            "name": "character_set_clip_stepped_fps",
            "description": "Configure quantized stepped playback FPS (e.g. 8 or 12 FPS on-twos/on-threes) on an animation clip, and optionally bake STEP keyframes for glTF 2.0 export.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "stepped_fps": { "type": ["number", "null"], "minimum": 1.0, "maximum": 60.0 },
                    "bake_stepped": { "type": "boolean" }
                }
            }
        }),
        json!({
            "name": "character_clamp_clip_to_floor",
            "description": "Sample in-between frames of an animation clip at fps and adjust the character Root bone's Y translation keyframes so the character stays flush on floor_y (default 0.0) or uneven surface meshes/voxels (surface_node_id / surface_node_ids, with overhang clearance, optional solve_foot_ik, align_foot_normal, pelvis_tilt, and custom contact_chains).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "bone_prefix": { "type": "string", "description": "Optional sub-skeleton prefix (e.g. 'P2_', default '' for primary character)" },
                    "floor_y": { "type": "number", "description": "Target flat floor Y height (default 0.0; fallback when outside surface)" },
                    "fps": { "type": "number", "minimum": 1, "maximum": 120, "description": "Sub-frame sampling rate (default 24)" },
                    "surface_node_id": { "type": "integer", "minimum": 1, "description": "Optional single terrain/surface mesh or voxel node_id for vertical -Y raycast grounding" },
                    "surface_node_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "description": "Optional list of multiple surface mesh/voxel node_ids queried simultaneously" },
                    "solve_foot_ik": { "type": "boolean", "description": "When true with surface target, solves two-bone leg IK on slopes (default false)" },
                    "align_foot_normal": { "type": "boolean", "description": "When true with surface target, tilts foot/end bones to match surface normal (default false)" },
                    "pelvis_tilt": { "type": "boolean", "description": "When true, rolls Root bone softly on steep left/right foot elevation deltas (default false)" },
                    "step_up_clearance": { "type": "number", "minimum": 0.05, "maximum": 20.0, "description": "Local raycast height above foot/root for overhangs/bridges/caves (default 1.0)" },
                    "footprint_half_length": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Half-length for heel+toe multi-ray footprint sweep (default 0.08; 0.0 = single ray)" },
                    "contact_chains": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 3, "maxItems": 3 },
                        "description": "Optional explicit [root_bone_id, mid_bone_id, end_bone_id] contact chains for quadruped/custom rigs"
                    }
                }
            }
        }),
        json!({
            "name": "character_clamp_to_floor",
            "description": "Automatically shift a character's Root bone Y position so its lowest skinned mesh vertex (or joint fallback) sits flush on floor_y (default 0.0) or uneven surface meshes/voxels (surface_node_id / surface_node_ids, with overhang clearance, optional solve_foot_ik, align_foot_normal, pelvis_tilt, and custom contact_chains).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "floor_y": { "type": "number", "description": "Target flat floor Y height (default 0.0; fallback when outside surface)" },
                    "bone_prefix": { "type": "string", "description": "Optional sub-skeleton prefix (e.g. 'P2_', default '' for primary character)" },
                    "surface_node_id": { "type": "integer", "minimum": 1, "description": "Optional single terrain/surface mesh or voxel node_id for vertical -Y raycast grounding" },
                    "surface_node_ids": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "description": "Optional list of multiple surface mesh/voxel node_ids queried simultaneously" },
                    "solve_foot_ik": { "type": "boolean", "description": "When true with surface target, solves two-bone leg IK on slopes (default false)" },
                    "align_foot_normal": { "type": "boolean", "description": "When true with surface target, tilts foot/end bones to match surface normal (default false)" },
                    "pelvis_tilt": { "type": "boolean", "description": "When true, rolls Root bone softly on steep left/right foot elevation deltas (default false)" },
                    "step_up_clearance": { "type": "number", "minimum": 0.05, "maximum": 20.0, "description": "Local raycast height above foot/root for overhangs/bridges/caves (default 1.0)" },
                    "footprint_half_length": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Half-length for heel+toe multi-ray footprint sweep (default 0.08; 0.0 = single ray)" },
                    "contact_chains": {
                        "type": "array",
                        "items": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "minItems": 3, "maxItems": 3 },
                        "description": "Optional explicit [root_bone_id, mid_bone_id, end_bone_id] contact chains for quadruped/custom rigs"
                    }
                }
            }
        }),
        json!({
            "name": "character_set_parent_switch_keyframe",
            "description": "Dynamically attach, switch parent bone, or detach/drop a prop or weapon node in world space mid-clip (e.g. hold rifle in RightHand at t=0.0, detach to floor world_position at t=0.5, or switch to Torso holster). Exports as standard glTF 2.0 TRS animation channels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "bone_id": { "type": ["integer", "null"], "description": "Bone ID to attach to, or null/omitted to detach into world space" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space position [x,y,z] when detached (bone_id=null), or local offset when attached" },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Euler XYZ degrees in world space when detached, or local offset when attached" }
                },
                "required": ["node_id", "time_secs"]
            }
        }),
        json!({
            "name": "character_set_visibility_keyframe",
            "description": "Insert or update a step-interpolated visibility keyframe (visible: true/false) for a scene node in an animation clip (ideal for toggling muzzle flashes, tracers, or props during playback).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "visible": { "type": "boolean" }
                },
                "required": ["node_id", "time_secs", "visible"]
            }
        }),
        json!({
            "name": "character_set_keyframe",
            "description": "Insert or update one or many bone TRS keyframes in an animation clip (creates clip if clip_name is new). Pass either a single keyframe (bone_id, time_secs, rotation_deg) OR bulk 'keyframes': [{ bone_id, time_secs, rotation_deg, translation?, scale?, interpolation? }, ...] in one atomic transaction. +X pitch tilts FORWARD, -X tilts BACKWARD in character-local space.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Delta offset [dx, dy, dz] relative to the bone's rest position" },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "interpolation": { "type": "string", "enum": ["linear", "step", "ease_in", "ease_out", "ease_in_out", "cubic_bezier"] },
                    "bezier": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "Optional [p1x, p1y, p2x, p2y] control points when interpolation is 'cubic_bezier'" },
                    "keyframes": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 2048,
                        "description": "Optional bulk list of keyframes to upsert atomically into the target clip",
                        "items": {
                            "type": "object",
                            "properties": {
                                "bone_id": { "type": "integer", "minimum": 0 },
                                "time_secs": { "type": "number", "minimum": 0.0 },
                                "rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                                "translation": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Delta offset [dx, dy, dz] relative to the bone's rest position" },
                                "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                                "interpolation": { "type": "string", "enum": ["linear", "step", "ease_in", "ease_out", "ease_in_out", "cubic_bezier"] },
                                "bezier": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                            },
                            "required": ["bone_id", "time_secs", "rotation_deg"]
                        }
                    }
                }
            }
        }),
        json!({
            "name": "character_solve_ik",
            "description": "Analytic two-bone IK: rotate root and mid bones so the end bone reaches target [x, y, z]. Use 'pole_target' for a world-space bend point OR 'pole_vector' for a world-space bend direction. Use 'end_world_rotation_deg' to orient end_bone_id in world space (auto-compensating forearm rotation) or 'end_rotation_deg' for local rotation. Returns reached position and resulting bone delta rotations_deg.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root_bone_id": { "type": "integer", "minimum": 0 },
                    "mid_bone_id": { "type": "integer", "minimum": 0 },
                    "end_bone_id": { "type": "integer", "minimum": 0 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space target position [x, y, z] for end_bone_id" },
                    "pole_target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space POINT [x, y, z] that the elbow/knee bends toward (auto-converted to direction from root_bone_id)" },
                    "pole_vector": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "World-space DIRECTION vector (not a world point) steering the elbow/knee bend plane; e.g. [0, -1, -1] for elbow down-back, [0, 0, 1] for knee forward" },
                    "end_world_rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Optional WORLD-SPACE Euler XYZ degrees for end_bone_id after solving IK (automatically compensates for mid_bone_id world rotation)" },
                    "end_rotation_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Optional LOCAL Euler XYZ degrees (relative to rest pose) applied to end_bone_id after solving IK" }
                },
                "required": ["root_bone_id", "mid_bone_id", "end_bone_id", "target"]
            }
        }),
        json!({
            "name": "character_snapshot_pose_to_keyframe",
            "description": "Bake/snapshot the current live bone pose (e.g. after character_solve_ik or character_clamp_to_floor) into an animation clip at time_secs as a single atomic transaction. Creates the clip automatically if clip_name is new. Supports optional interpolation ('linear'|'step'|'ease_in'|'ease_out'|'ease_in_out'|'cubic_bezier') and 'bezier': [p1x,p1y,p2x,p2y].",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "bone_ids": { "type": "array", "items": { "type": "integer", "minimum": 0 }, "description": "Optional subset of bone IDs to snapshot (defaults to all bones in the rig)" },
                    "interpolation": { "type": "string", "enum": ["linear", "step", "ease_in", "ease_out", "ease_in_out", "cubic_bezier"] },
                    "bezier": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 }
                },
                "required": ["time_secs"]
            }
        }),
        json!({
            "name": "character_set_morph_weight",
            "description": "Insert or update a morph-target weight keyframe on a mesh node in an animation clip (creates clip if clip_name is new). Animate blend shapes over time; export writes glTF weight channels.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "target": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "weight": { "type": "number" }
                },
                "required": ["node_id", "target", "time_secs", "weight"]
            }
        }),
        json!({
            "name": "character_remove_morph_weight",
            "description": "Remove a morph-target weight keyframe matching time_secs (within 1ms) from an animation clip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "node_id": { "type": "integer", "minimum": 1 },
                    "target": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["clip_index", "node_id", "target", "time_secs"]
            }
        }),
        json!({
            "name": "character_list_bones",
            "description": "List all bones of the character rig (id, name, parent, local_rest_position, world_rest_position, and coordinate_space). Fails when the scene has no rig.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "character_inspect_weights",
            "description": "Inspect skinning bone influences at a 3D world point [x,y,z] and/or for a scene node_id (reports rigid binding, manual weight count, and resolved bone names + weights).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "minimum": 1 },
                    "point": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                }
            }
        }),
        json!({
            "name": "character_inspect_clip",
            "description": "Inspect an animation clip in full: keyframes per bone channel (time, translation, rotation quat + degrees, scale) and morph weight tracks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_sample_pose",
            "description": "Sample an animation clip at time_secs without touching live pose. Returns per-bone world positions and local XYZ-euler degrees.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["time_secs"]
            }
        }),
        json!({
            "name": "character_set_active_clip",
            "description": "Set the active animation clip (viewport playback target).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_set_playback_time",
            "description": "Scrub an animation clip to time_secs and/or toggle live viewport play/pause (playing: true/false). Immediately applies bone poses and morph weights in all 3D modes (lowpoly, voxel, character) for live playback or viewport_screenshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "playing": { "type": "boolean", "description": "Optional live viewport play/pause state (defaults to false when scrubbing)" }
                }
            }
        }),
        json!({
            "name": "character_reset_pose",
            "description": "Reset all bones to their rest pose and clear live morph weights in a single undoable command.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "character_remove_keyframe",
            "description": "Remove a bone keyframe matching time_secs (within 1ms) from an animation clip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 }
                },
                "required": ["bone_id", "time_secs"]
            }
        }),
        json!({
            "name": "character_remove_clip",
            "description": "Remove an animation clip by index or name (active clip index is repaired when needed).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "clip_name": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "character_remove_bone",
            "description": "Remove a bone and its whole subtree (children, bindings, and bone channels referencing it). Reports counts of removed items.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 }
                },
                "required": ["bone_id"]
            }
        }),
        json!({
            "name": "character_create_bone_grid",
            "description": "Create an M x N multi-chain cloth bone grid (each column is a parent-child chain from pinned top row to free tip) and optionally assign its bone_mask to a mesh node.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prefix": { "type": "string", "description": "Bone name prefix (default 'Cloth')" },
                    "cols": { "type": "integer", "minimum": 2, "maximum": 16 },
                    "rows": { "type": "integer", "minimum": 2, "maximum": 16 },
                    "origin": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "width": { "type": "number", "minimum": 0.01 },
                    "height": { "type": "number", "minimum": 0.01 },
                    "axis_u": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "axis_v": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "parent_bone": { "type": "integer", "minimum": 0 },
                    "bind_node": { "type": "integer", "minimum": 1 }
                }
            }
        }),
        json!({
            "name": "character_bake_cloth_wind",
            "description": "Deterministically bake kinematic wind/gravity ripple animation for an existing cloth bone grid (by prefix) into a glTF-exportable AnimationClip.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_prefix": { "type": "string", "description": "Prefix used in character_create_bone_grid (e.g. 'Cloth')" },
                    "clip_name": { "type": "string" },
                    "wind_dir": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "wind_strength": { "type": "number" },
                    "gravity": { "type": "number" },
                    "turbulence": { "type": "number" },
                    "phase_offset": { "type": "number" },
                    "duration_secs": { "type": "number", "minimum": 0.1 },
                    "fps": { "type": "number", "minimum": 1.0, "maximum": 60.0 }
                },
                "required": ["bone_prefix"]
            }
        }),
        json!({
            "name": "character_stick_gizmo_pose",
            "description": "Apply an octahedral Stick Gizmo bone pose operation (roll, pitch, yaw, twist, roll_twist) with optional snap_deg (5/15/45), mirror_lr, and axis_lock (0=X,1=Y,2=Z) via Command Bus.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "mode": { "type": "string", "enum": ["roll", "pitch", "yaw", "twist", "roll_twist"] },
                    "angle_deg": { "type": "number" },
                    "chain_len": { "type": "integer", "minimum": 1, "maximum": 32 },
                    "snap_deg": { "type": "number", "minimum": 0.0 },
                    "mirror_lr": { "type": "boolean" },
                    "axis_lock": { "type": "integer", "minimum": 0, "maximum": 2 }
                },
                "required": ["bone_id", "mode", "angle_deg"]
            }
        }),
        json!({
            "name": "character_set_bone_scale",
            "description": "Set a bone's local squash-and-stretch pose scale [sx, sy, sz] (clamped >= 1e-6 anti-zero, v5.5).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["bone_id", "scale"]
            }
        }),
        json!({
            "name": "character_set_bone_properties",
            "description": "Configure bone rotation limits (min_deg/max_deg), inherit_rotation, inherit_scale, connected, deform (false excludes helper bones from glTF skinning), spring_stiffness/damping, or lod_level (v5.5-v5.7).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "min_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "max_deg": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "inherit_rotation": { "type": "boolean" },
                    "inherit_scale": { "type": "boolean" },
                    "connected": { "type": "boolean" },
                    "deform": { "type": "boolean" },
                    "spring_stiffness": { "type": "number", "minimum": 0.0, "maximum": 200.0 },
                    "spring_damping": { "type": "number", "minimum": 0.01, "maximum": 50.0 },
                    "lod_level": { "type": "integer", "minimum": 0, "maximum": 8 }
                },
                "required": ["bone_id"]
            }
        }),
        json!({
            "name": "character_set_constraint",
            "description": "Upsert a deterministic rig constraint (ik_fk_blend, look_at, or copy_transform) and evaluate it onto the active rig (v5.6).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["ik_fk_blend", "look_at", "copy_transform"] },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "source_bone_id": { "type": "integer", "minimum": 0 },
                    "root": { "type": "integer", "minimum": 0 },
                    "mid": { "type": "integer", "minimum": 0 },
                    "end": { "type": "integer", "minimum": 0 },
                    "target": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "pole": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "weight": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                },
                "required": ["name", "kind"]
            }
        }),
        json!({
            "name": "character_set_socket",
            "description": "Upsert a named attachment socket on a bone with optional local offset position [x,y,z] (exported in glTF extras, v5.6).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "bone_id": { "type": "integer", "minimum": 0 },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 }
                },
                "required": ["name", "bone_id"]
            }
        }),
        json!({
            "name": "character_pose_preset",
            "description": "Save a named pose preset, apply a saved pose (optionally X-flipped), or mirror the current Left<->Right rig pose (v5.5/v5.7).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["save", "apply", "mirror"] },
                    "name": { "type": "string" },
                    "flipped": { "type": "boolean" },
                    "left_to_right": { "type": "boolean" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "character_bake_spring_bones",
            "description": "Deterministically bake procedural spring/jiggle motion for all bones with spring_stiffness > 0 into an animation clip (v5.7).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "fps": { "type": "number", "minimum": 1.0, "maximum": 120.0 }
                }
            }
        }),
        json!({
            "name": "character_add_event",
            "description": "Add a timeline event/notify marker (e.g. footstep SFX or attack hitbox) to an animation clip (exported in glTF extras, v5.7).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "clip_index": { "type": "integer", "minimum": 0 },
                    "time_secs": { "type": "number", "minimum": 0.0 },
                    "name": { "type": "string" },
                    "payload": { "type": "string" }
                },
                "required": ["time_secs", "name"]
            }
        }),
        json!({
            "name": "character_apply_foot_roll",
            "description": "Apply single-control reverse foot roll across heel, ball, and toe bones (v5.7).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "heel_id": { "type": "integer", "minimum": 0 },
                    "ball_id": { "type": "integer", "minimum": 0 },
                    "toe_id": { "type": "integer", "minimum": 0 },
                    "roll_deg": { "type": "number", "minimum": -60.0, "maximum": 90.0 }
                },
                "required": ["heel_id", "ball_id", "toe_id", "roll_deg"]
            }
        }),
    ]
}
