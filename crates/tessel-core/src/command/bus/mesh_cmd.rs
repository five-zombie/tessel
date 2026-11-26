#![allow(unused_imports, unused_variables)]
use glam::Quat;
use serde::{Deserialize, Serialize};

use crate::character::{
    AnimationClip, Bone, BoneId, CharacterRig, ClipLayer, Keyframe, SkinBinding,
};
use crate::error::CoreError;
use crate::mesh::{
    ops as mesh_ops, FaceId, LowPolyMesh, MeshMaterial, MeshPaintLayer, MeshTexture, PrimitiveKind,
    UvProjection, DEFAULT_MESH_TEX_SIZE,
};
use crate::palette::{PaletteEntry, PaletteIndex, Rgba8};
use crate::pixel::{FrameIndex, LayerId, PixelCanvas, PixelEdit, PixelFrame, PixelLayer};
use crate::scene::{AppMode, NodeId, NodeKind, SceneNode, SceneState, Transform};
use crate::voxel::{ops as voxel_ops, VoxelCoord, VoxelEdit, VoxelVolume};

use super::{insert_validated_scene_node, Command, UndoDelta};

pub(super) fn apply(cmd: &Command, scene: &mut SceneState) -> Result<UndoDelta, CoreError> {
    let delta = match cmd {
        Command::CreatePrimitiveMesh {
            name,
            kind,
            size,
            material,
        } => {
            let mesh = mesh_ops::create_primitive(*kind, *size, *material);
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::LowPolyMesh,
                None,
                Transform::default(),
            )?;
            scene.lowpoly_meshes.insert(id, mesh);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::CreateTerrainMesh {
            name,
            params,
            transform,
        } => {
            let mesh = mesh_ops::create_terrain_mesh(params)?;
            let (id, previous_selection) =
                insert_validated_scene_node(scene, name, NodeKind::LowPolyMesh, None, *transform)?;
            scene.lowpoly_meshes.insert(id, mesh);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::CreateCurveTubeMesh {
            name,
            points,
            radius,
            flatness,
            material,
        } => {
            let mesh = mesh_ops::create_curve_tube(points, *radius, *flatness, *material)?;
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::LowPolyMesh,
                None,
                Transform::default(),
            )?;
            scene.lowpoly_meshes.insert(id, mesh);
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::ImportLowPolyMesh { name, mesh } => {
            let (id, previous_selection) = insert_validated_scene_node(
                scene,
                name,
                NodeKind::LowPolyMesh,
                None,
                Transform::default(),
            )?;
            scene.lowpoly_meshes.insert(id, mesh.clone());
            UndoDelta::RemoveNode {
                id,
                previous_selection,
            }
        }
        Command::ExtrudeMeshFace {
            node_id,
            face_id,
            distance,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::extrude_face(&current, *face_id, *distance)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::InsetMeshFace {
            node_id,
            face_id,
            factor,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::inset_face(&current, *face_id, *factor)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::TranslateMeshFace {
            node_id,
            face_id,
            delta,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::translate_face(&current, *face_id, *delta)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::MoveMeshVertices {
            node_id,
            vertex_ids,
            delta,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::move_mesh_vertices(&current, vertex_ids, *delta)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshVertex {
            node_id,
            vertex_id,
            position,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_mesh_vertex(&current, *vertex_id, *position)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshVerticesBatch { node_id, updates } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_vertices_batch(&current, updates)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshFaceMaterials {
            node_id,
            face_ids,
            material,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_face_materials(&current, face_ids, *material)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::MirrorLowPolyMesh {
            node_id,
            axis,
            merge_original,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::mirror_mesh(&current, *axis, *merge_original)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::TaperLowPolyMesh {
            node_id,
            axis,
            start_scale,
            end_scale,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::taper_mesh(&current, *axis, *start_scale, *end_scale)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::BendLowPolyMesh {
            node_id,
            bend_axis,
            curve_axis,
            angle_rad,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::bend_mesh(&current, *bend_axis, *curve_axis, *angle_rad)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshOrigin {
            node_id,
            origin_local,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_mesh_origin(&current, *origin_local)?;
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous_transform = node.transform;
            let world_shift = node.transform.rotation * (node.transform.scale * *origin_local);
            node.transform.translation += world_shift;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::Transaction(vec![
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                },
                UndoDelta::RestoreNodeTransform {
                    node_id: *node_id,
                    previous: previous_transform,
                },
            ])
        }
        Command::ApplyMeshTransform { node_id } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let node = scene
                .nodes
                .get_mut(node_id)
                .ok_or(CoreError::NodeNotFound(*node_id))?;
            let previous_transform = node.transform;
            let baked = mesh_ops::apply_transform(&current, &previous_transform)?;
            node.transform = Transform::default();
            scene.lowpoly_meshes.insert(*node_id, baked);
            UndoDelta::Transaction(vec![
                UndoDelta::RestoreLowPolyMesh {
                    node_id: *node_id,
                    previous_mesh: current,
                },
                UndoDelta::RestoreNodeTransform {
                    node_id: *node_id,
                    previous: previous_transform,
                },
            ])
        }
        Command::WeldMeshVertices { node_id, tolerance } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let (next, _) = mesh_ops::weld_vertices(&current, *tolerance)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::FlipMeshNormals { node_id, face_ids } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::flip_normals(&current, face_ids.as_deref())?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::JoinLowPolyMeshes {
            target_node_id,
            source_node_ids,
        } => {
            if source_node_ids.is_empty() {
                return Err(CoreError::OutOfBounds);
            }
            let target_mesh = scene
                .lowpoly_meshes
                .get(target_node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*target_node_id))?
                .clone();
            let target_world = scene.node_world_matrix(*target_node_id);
            if target_world.determinant().abs() <= 1e-8 {
                return Err(CoreError::OutOfBounds);
            }
            let target_inv = target_world.inverse();
            let mut owned_sources: Vec<(NodeId, LowPolyMesh, glam::Mat4)> =
                Vec::with_capacity(source_node_ids.len());
            for &src_id in source_node_ids {
                if src_id == *target_node_id {
                    continue;
                }
                let src_mesh = scene
                    .lowpoly_meshes
                    .get(&src_id)
                    .ok_or(CoreError::LowPolyMeshNotFound(src_id))?
                    .clone();
                let src_world = scene.node_world_matrix(src_id);
                owned_sources.push((src_id, src_mesh, src_world));
            }
            if owned_sources.is_empty() {
                return Err(CoreError::OutOfBounds);
            }
            let mut parts: Vec<(&LowPolyMesh, glam::Mat4)> =
                Vec::with_capacity(1 + owned_sources.len());
            parts.push((&target_mesh, target_world));
            for (_, m, w) in &owned_sources {
                parts.push((m, *w));
            }
            let merged = mesh_ops::join_meshes(&parts, target_inv)?;
            scene.lowpoly_meshes.insert(*target_node_id, merged);
            let mut deltas = Vec::with_capacity(1 + owned_sources.len());
            deltas.push(UndoDelta::RestoreLowPolyMesh {
                node_id: *target_node_id,
                previous_mesh: target_mesh,
            });
            for (src_id, _, _) in owned_sources {
                let del = Command::DeleteNode(src_id).apply(scene)?;
                deltas.push(del);
            }
            scene.selected_node = Some(*target_node_id);
            UndoDelta::Transaction(deltas)
        }
        Command::SetMeshFaceUvs {
            node_id,
            face_id,
            uvs,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_face_uvs(&current, *face_id, uvs)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshVertexWeights {
            node_id,
            vertex_ids,
            joints,
            weights,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::set_vertex_weights(&current, vertex_ids, *joints, *weights)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::PaintMeshWeights {
            node_id,
            hit_point,
            radius,
            bone_id,
            strength,
        } => {
            let rig = scene
                .character_rig
                .as_ref()
                .ok_or(CoreError::CharacterRigNotFound)?;
            let joint_idx = rig
                .bones
                .iter()
                .position(|b| b.id == *bone_id)
                .and_then(|i| u16::try_from(i).ok())
                .ok_or(CoreError::OutOfBounds)?;
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let solver = crate::character::AutoSkinSolver::from_rig(rig, 1.0);
            let rest_mat = scene.node_rest_world_matrix(*node_id);
            let fallback = |_vi: usize, local_pos: glam::Vec3| {
                if let Some(ref s) = solver {
                    let world_pos = rest_mat.transform_point3(local_pos);
                    s.weights_for_vertex(Some(*node_id), world_pos)
                } else {
                    crate::character::VertexSkinWeights {
                        joints: [joint_idx, 0, 0, 0],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    }
                }
            };
            let Some(next) = mesh_ops::paint_vertex_weights_brush(
                &current,
                glam::Vec3::from_array(*hit_point),
                *radius,
                joint_idx,
                *strength,
                Some(&fallback),
            )?
            else {
                return Err(CoreError::OutOfBounds);
            };
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::ClearMeshVertexWeights {
            node_id,
            vertex_ids,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::clear_vertex_weights(&current, vertex_ids.as_deref())?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::AddMorphDelta {
            node_id,
            name,
            vertex_ids,
            delta,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::add_morph_delta(&current, name, vertex_ids, *delta)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::ClearMorphs { node_id, name } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::clear_morphs(&current, name.as_deref())?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SmoothSubdivideMesh { node_id } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::smooth_subdivide_mesh(&current)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::BevelMesh { node_id, amount } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::bevel_mesh(&current, *amount)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::DecimateMesh { node_id, ratio } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::decimate_mesh(&current, *ratio)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMeshSmoothShading { node_id, smooth } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = mesh.smooth_shading;
            mesh.smooth_shading = *smooth;
            UndoDelta::RestoreMeshSmoothShading {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetMeshMaterial { node_id, material } => {
            if !material.metallic.is_finite()
                || !material.roughness.is_finite()
                || material.emissive.iter().any(|c| !c.is_finite())
            {
                return Err(CoreError::OutOfBounds);
            }
            let sanitized = MeshMaterial {
                metallic: material.metallic.clamp(0.0, 1.0),
                roughness: material.roughness.clamp(0.05, 1.0),
                emissive: [
                    material.emissive[0].clamp(0.0, 1.0),
                    material.emissive[1].clamp(0.0, 1.0),
                    material.emissive[2].clamp(0.0, 1.0),
                ],
                double_sided: material.double_sided,
                cast_shadow: material.cast_shadow,
                receive_shadow: material.receive_shadow,
                alpha_mode: match material.alpha_mode {
                    crate::mesh::AlphaMode::Mask { cutoff } if cutoff.is_finite() => {
                        crate::mesh::AlphaMode::Mask {
                            cutoff: cutoff.clamp(0.0, 1.0),
                        }
                    }
                    crate::mesh::AlphaMode::Mask { .. } => {
                        crate::mesh::AlphaMode::Mask { cutoff: 0.5 }
                    }
                    other => other,
                },
                unlit: material.unlit,
            };
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = mesh.material;
            mesh.material = sanitized;
            UndoDelta::RestoreMeshMaterial {
                node_id: *node_id,
                previous,
            }
        }
        Command::PaintMeshTexture {
            node_id,
            uv,
            radius_px,
            color,
        } => {
            let base_color = scene
                .lowpoly_meshes
                .get(node_id)
                .and_then(|m| m.faces.first())
                .and_then(|f| scene.palette.get(f.material))
                .map(|e| e.color)
                .unwrap_or(Rgba8::rgb(225, 226, 235));
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if let Some(layer_idx) = mesh.active_paint_layer_index() {
                let layer_id = mesh.paint_layers[layer_idx].id;
                let previous_pixels = mesh.paint_layers[layer_idx]
                    .texture
                    .paint_disc(*uv, *radius_px, *color);
                mesh.recomposite_paint_layers();
                UndoDelta::RestoreMeshPaintLayerPixels {
                    node_id: *node_id,
                    layer_id,
                    previous_pixels,
                }
            } else {
                let was_none = mesh.texture.is_none();
                let tex = mesh.texture.get_or_insert_with(|| {
                    MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                });
                let previous_pixels = tex.paint_disc(*uv, *radius_px, *color);
                UndoDelta::RestoreMeshTexturePixels {
                    node_id: *node_id,
                    was_none,
                    previous_pixels,
                }
            }
        }
        Command::PaintMeshSpot3D {
            node_id,
            center,
            radius,
            color,
            brush_normal,
            camera_pos,
        } => {
            let base_color = scene
                .lowpoly_meshes
                .get(node_id)
                .and_then(|m| m.faces.first())
                .and_then(|f| scene.palette.get(f.material))
                .map(|e| e.color)
                .unwrap_or(Rgba8::rgb(225, 226, 235));
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let params = crate::mesh::PaintSpot3DParams {
                center: *center,
                radius: *radius,
                color: *color,
                brush_normal: *brush_normal,
                camera_pos: *camera_pos,
            };
            if let Some(layer_idx) = mesh.active_paint_layer_index() {
                let layer_id = mesh.paint_layers[layer_idx].id;
                let mut tex = std::mem::replace(
                    &mut mesh.paint_layers[layer_idx].texture,
                    MeshTexture {
                        width: 0,
                        height: 0,
                        rgba: Vec::new(),
                    },
                );
                let previous_pixels = crate::mesh::paint_mesh_spot_3d(mesh, &mut tex, &params);
                mesh.paint_layers[layer_idx].texture = tex;
                mesh.recomposite_paint_layers();
                UndoDelta::RestoreMeshPaintLayerPixels {
                    node_id: *node_id,
                    layer_id,
                    previous_pixels,
                }
            } else {
                let was_none = mesh.texture.is_none();
                let mut tex = mesh.texture.take().unwrap_or_else(|| {
                    MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                });
                let previous_pixels = crate::mesh::paint_mesh_spot_3d(mesh, &mut tex, &params);
                mesh.texture = Some(tex);
                UndoDelta::RestoreMeshTexturePixels {
                    node_id: *node_id,
                    was_none,
                    previous_pixels,
                }
            }
        }
        Command::PaintMeshLine {
            node_id,
            uv_start,
            uv_end,
            width_px,
            color,
        } => {
            let base_color = scene
                .lowpoly_meshes
                .get(node_id)
                .and_then(|m| m.faces.first())
                .and_then(|f| scene.palette.get(f.material))
                .map(|e| e.color)
                .unwrap_or(Rgba8::rgb(225, 226, 235));
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if let Some(layer_idx) = mesh.active_paint_layer_index() {
                let layer_id = mesh.paint_layers[layer_idx].id;
                let previous_pixels = mesh.paint_layers[layer_idx]
                    .texture
                    .paint_line(*uv_start, *uv_end, *width_px, *color);
                mesh.recomposite_paint_layers();
                UndoDelta::RestoreMeshPaintLayerPixels {
                    node_id: *node_id,
                    layer_id,
                    previous_pixels,
                }
            } else {
                let was_none = mesh.texture.is_none();
                let tex = mesh.texture.get_or_insert_with(|| {
                    MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                });
                let previous_pixels = tex.paint_line(*uv_start, *uv_end, *width_px, *color);
                UndoDelta::RestoreMeshTexturePixels {
                    node_id: *node_id,
                    was_none,
                    previous_pixels,
                }
            }
        }
        Command::PaintMeshRect {
            node_id,
            uv_min,
            uv_max,
            color,
        } => {
            let base_color = scene
                .lowpoly_meshes
                .get(node_id)
                .and_then(|m| m.faces.first())
                .and_then(|f| scene.palette.get(f.material))
                .map(|e| e.color)
                .unwrap_or(Rgba8::rgb(225, 226, 235));
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if let Some(layer_idx) = mesh.active_paint_layer_index() {
                let layer_id = mesh.paint_layers[layer_idx].id;
                let previous_pixels = mesh.paint_layers[layer_idx]
                    .texture
                    .paint_rect(*uv_min, *uv_max, *color);
                mesh.recomposite_paint_layers();
                UndoDelta::RestoreMeshPaintLayerPixels {
                    node_id: *node_id,
                    layer_id,
                    previous_pixels,
                }
            } else {
                let was_none = mesh.texture.is_none();
                let tex = mesh.texture.get_or_insert_with(|| {
                    MeshTexture::solid(DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE, base_color)
                });
                let previous_pixels = tex.paint_rect(*uv_min, *uv_max, *color);
                UndoDelta::RestoreMeshTexturePixels {
                    node_id: *node_id,
                    was_none,
                    previous_pixels,
                }
            }
        }
        Command::ProjectMeshUvs { node_id, mode } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::project_uvs(&current, *mode)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SubdivideMeshFace { node_id, face_id } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let next = mesh_ops::subdivide_face(&current, *face_id)?;
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::FillMeshTexture { node_id, color } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if let Some(layer_idx) = mesh.active_paint_layer_index() {
                let previous_layers = mesh.paint_layers.clone();
                let previous_active = mesh.active_paint_layer;
                let previous_texture = mesh.texture.clone();
                mesh.paint_layers[layer_idx].texture.fill(*color);
                mesh.recomposite_paint_layers();
                UndoDelta::RestoreMeshPaintLayers {
                    node_id: *node_id,
                    previous_layers,
                    previous_active,
                    previous_texture,
                }
            } else {
                let previous = mesh.texture.clone();
                match &mut mesh.texture {
                    Some(tex) => tex.fill(*color),
                    None => {
                        mesh.texture = Some(MeshTexture::solid(
                            DEFAULT_MESH_TEX_SIZE,
                            DEFAULT_MESH_TEX_SIZE,
                            *color,
                        ));
                    }
                }
                UndoDelta::RestoreMeshTextureFull {
                    node_id: *node_id,
                    previous,
                }
            }
        }
        Command::SetMeshTexture { node_id, texture } => {
            if let Some(tex) = texture {
                let expected = (tex.width as usize)
                    .checked_mul(tex.height as usize)
                    .and_then(|px| px.checked_mul(4));
                if tex.width == 0
                    || tex.height == 0
                    || tex.width > 1024
                    || tex.height > 1024
                    || expected != Some(tex.rgba.len())
                {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if !mesh.paint_layers.is_empty() {
                let previous_layers = std::mem::take(&mut mesh.paint_layers);
                let previous_active = mesh.active_paint_layer.take();
                let previous_texture = mesh.texture.clone();
                mesh.texture = texture.clone();
                UndoDelta::RestoreMeshPaintLayers {
                    node_id: *node_id,
                    previous_layers,
                    previous_active,
                    previous_texture,
                }
            } else {
                let previous = mesh.texture.clone();
                mesh.texture = texture.clone();
                UndoDelta::RestoreMeshTextureFull {
                    node_id: *node_id,
                    previous,
                }
            }
        }
        Command::SetMeshNormalMap {
            node_id,
            normal_map,
        } => {
            if let Some(tex) = normal_map {
                let expected = (tex.width as usize)
                    .checked_mul(tex.height as usize)
                    .and_then(|px| px.checked_mul(4));
                if tex.width == 0
                    || tex.height == 0
                    || tex.width > 1024
                    || tex.height > 1024
                    || expected != Some(tex.rgba.len())
                {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = mesh.normal_map.clone();
            mesh.normal_map = normal_map.clone();
            UndoDelta::RestoreMeshNormalMap {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetMeshAoMap { node_id, ao_map } => {
            if let Some(tex) = ao_map {
                let expected = (tex.width as usize)
                    .checked_mul(tex.height as usize)
                    .and_then(|px| px.checked_mul(4));
                if tex.width == 0
                    || tex.height == 0
                    || tex.width > 1024
                    || tex.height > 1024
                    || expected != Some(tex.rgba.len())
                {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = mesh.ao_map.clone();
            mesh.ao_map = ao_map.clone();
            UndoDelta::RestoreMeshAoMap {
                node_id: *node_id,
                previous,
            }
        }
        Command::AddMeshPaintLayer {
            node_id,
            name,
            opacity,
            fill_color,
        } => {
            if !opacity.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let base_color = scene
                .lowpoly_meshes
                .get(node_id)
                .and_then(|m| m.faces.first())
                .and_then(|f| scene.palette.get(f.material))
                .map(|e| e.color)
                .unwrap_or(Rgba8::rgb(225, 226, 235));
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let added_slots = if mesh.paint_layers.is_empty() { 2 } else { 1 };
            if mesh.paint_layers.len().saturating_add(added_slots)
                > crate::mesh::layer::MAX_PAINT_LAYERS
            {
                return Err(CoreError::OutOfBounds);
            }
            let previous_layers = mesh.paint_layers.clone();
            let previous_active = mesh.active_paint_layer;
            let previous_texture = mesh.texture.clone();

            let (w, h) = mesh
                .paint_layers
                .first()
                .map(|l| (l.texture.width, l.texture.height))
                .or_else(|| mesh.texture.as_ref().map(|t| (t.width, t.height)))
                .unwrap_or((DEFAULT_MESH_TEX_SIZE, DEFAULT_MESH_TEX_SIZE));

            if mesh.paint_layers.is_empty() {
                let base_tex = mesh
                    .texture
                    .clone()
                    .unwrap_or_else(|| MeshTexture::solid(w, h, base_color));
                mesh.paint_layers
                    .push(MeshPaintLayer::new(1, "Base", 1.0, base_tex));
            }

            let new_id = mesh.next_paint_layer_id();
            let fill = fill_color.unwrap_or(Rgba8::rgba(0, 0, 0, 0));
            let layer_tex = MeshTexture::solid(w, h, fill);
            let label = if name.trim().is_empty() {
                format!("Layer {new_id}")
            } else {
                name.trim().to_string()
            };
            mesh.paint_layers
                .push(MeshPaintLayer::new(new_id, label, *opacity, layer_tex));
            mesh.active_paint_layer = Some(new_id);
            mesh.recomposite_paint_layers();

            UndoDelta::RestoreMeshPaintLayers {
                node_id: *node_id,
                previous_layers,
                previous_active,
                previous_texture,
            }
        }
        Command::SetMeshPaintLayer {
            node_id,
            layer_id,
            name,
            visible,
            opacity,
            set_active,
        } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let Some(pos) = mesh.paint_layers.iter().position(|l| l.id == *layer_id) else {
                return Err(CoreError::OutOfBounds);
            };
            if let Some(op) = opacity {
                if !op.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous_layers = mesh.paint_layers.clone();
            let previous_active = mesh.active_paint_layer;
            let previous_texture = mesh.texture.clone();

            if let Some(n) = name {
                if !n.trim().is_empty() {
                    mesh.paint_layers[pos].name = n.trim().to_string();
                }
            }
            if let Some(v) = visible {
                mesh.paint_layers[pos].visible = *v;
            }
            if let Some(op) = opacity {
                mesh.paint_layers[pos].opacity = op.clamp(0.0, 1.0);
            }
            if *set_active {
                mesh.active_paint_layer = Some(*layer_id);
            }
            mesh.recomposite_paint_layers();

            UndoDelta::RestoreMeshPaintLayers {
                node_id: *node_id,
                previous_layers,
                previous_active,
                previous_texture,
            }
        }
        Command::DeleteMeshPaintLayer { node_id, layer_id } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let Some(pos) = mesh.paint_layers.iter().position(|l| l.id == *layer_id) else {
                return Err(CoreError::OutOfBounds);
            };
            let previous_layers = mesh.paint_layers.clone();
            let previous_active = mesh.active_paint_layer;
            let previous_texture = mesh.texture.clone();

            mesh.paint_layers.remove(pos);
            if mesh.active_paint_layer == Some(*layer_id) {
                mesh.active_paint_layer = mesh.paint_layers.last().map(|l| l.id);
            }
            if mesh.paint_layers.is_empty() {
                mesh.texture = None;
            } else {
                mesh.recomposite_paint_layers();
            }

            UndoDelta::RestoreMeshPaintLayers {
                node_id: *node_id,
                previous_layers,
                previous_active,
                previous_texture,
            }
        }
        Command::SetMeshVertexColors { node_id, updates } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            for &(vid, rgba) in updates {
                if vid >= mesh.vertices.len() || rgba.iter().any(|c| !c.is_finite()) {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous = mesh.vertex_colors.clone();
            for &(vid, rgba) in updates {
                mesh.vertex_colors.insert(
                    vid,
                    [
                        rgba[0].clamp(0.0, 1.0),
                        rgba[1].clamp(0.0, 1.0),
                        rgba[2].clamp(0.0, 1.0),
                        rgba[3].clamp(0.0, 1.0),
                    ],
                );
            }
            UndoDelta::RestoreMeshVertexColors {
                node_id: *node_id,
                previous,
            }
        }
        Command::ClearMeshVertexColors { node_id } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = std::mem::take(&mut mesh.vertex_colors);
            UndoDelta::RestoreMeshVertexColors {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetMeshProxyNormals {
            node_id,
            normals,
            clear,
        } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            for &(vid, n) in normals {
                if vid >= mesh.vertices.len() || !n.is_finite() || n.length_squared() < 1e-8 {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous = mesh.proxy_normals.clone();
            if *clear {
                mesh.proxy_normals.clear();
            }
            for &(vid, n) in normals {
                mesh.proxy_normals.insert(vid, n.normalize());
            }
            UndoDelta::RestoreMeshProxyNormals {
                node_id: *node_id,
                previous,
            }
        }
        Command::SetMeshCreaseEdges { node_id, edges } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            for &(a, b) in edges {
                if a >= mesh.vertices.len() || b >= mesh.vertices.len() || a == b {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let previous = mesh.crease_edges.clone();
            mesh.crease_edges = edges.clone();
            UndoDelta::RestoreMeshCreaseEdges {
                node_id: *node_id,
                previous,
            }
        }
        Command::ClearMeshTexture {
            node_id,
            clear_color,
            clear_normal,
        } => {
            if !clear_color && !clear_normal {
                return Err(CoreError::OutOfBounds);
            }
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let mut deltas = Vec::with_capacity(2);
            if *clear_color {
                if !mesh.paint_layers.is_empty() {
                    let previous_layers = std::mem::take(&mut mesh.paint_layers);
                    let previous_active = mesh.active_paint_layer.take();
                    let previous_texture = mesh.texture.take();
                    deltas.push(UndoDelta::RestoreMeshPaintLayers {
                        node_id: *node_id,
                        previous_layers,
                        previous_active,
                        previous_texture,
                    });
                } else {
                    let previous = mesh.texture.take();
                    deltas.push(UndoDelta::RestoreMeshTextureFull {
                        node_id: *node_id,
                        previous,
                    });
                }
            }
            if *clear_normal {
                let previous = mesh.normal_map.take();
                deltas.push(UndoDelta::RestoreMeshNormalMap {
                    node_id: *node_id,
                    previous,
                });
            }
            let mut iter = deltas.into_iter();
            match (iter.next(), iter.next()) {
                (Some(one), None) => one,
                (Some(a), Some(b)) => UndoDelta::Transaction(vec![a, b]),
                _ => return Err(CoreError::OutOfBounds),
            }
        }
        Command::SetMorphTargetDeltas {
            node_id,
            name,
            position_deltas,
            normal_deltas,
            replace,
        } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(CoreError::OutOfBounds);
            }
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let mut next = current.clone();
            let vcount = next.vertices.len();
            for &(vi, d) in position_deltas {
                if vi >= vcount || !d.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
            }
            for &(vi, d) in normal_deltas {
                if vi >= vcount || !d.is_finite() {
                    return Err(CoreError::OutOfBounds);
                }
            }
            let target = if let Some(t) = next.morphs.iter_mut().find(|m| m.name == trimmed) {
                if *replace {
                    t.deltas.clear();
                    t.normal_deltas.clear();
                }
                t
            } else {
                next.morphs.push(crate::mesh::MorphTarget {
                    name: trimmed.to_string(),
                    deltas: std::collections::BTreeMap::new(),
                    normal_deltas: std::collections::BTreeMap::new(),
                    default_weight: 0.0,
                });
                next.morphs.last_mut().ok_or(CoreError::OutOfBounds)?
            };
            for &(vi, d) in position_deltas {
                if *replace {
                    if d.length_squared() > 1e-10 {
                        target.deltas.insert(vi, d);
                    }
                } else {
                    let entry = target.deltas.entry(vi).or_insert(glam::Vec3::ZERO);
                    *entry += d;
                }
            }
            for &(vi, d) in normal_deltas {
                if *replace {
                    if d.length_squared() > 1e-10 {
                        target.normal_deltas.insert(vi, d);
                    }
                } else {
                    let entry = target.normal_deltas.entry(vi).or_insert(glam::Vec3::ZERO);
                    *entry += d;
                }
            }
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::SetMorphDefaultWeight {
            node_id,
            name,
            weight,
        } => {
            if !weight.is_finite() {
                return Err(CoreError::OutOfBounds);
            }
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let mut next = current.clone();
            let target = next
                .morphs
                .iter_mut()
                .find(|m| m.name == name.as_str())
                .ok_or(CoreError::OutOfBounds)?;
            let clamped = weight.clamp(0.0, 1.0);
            target.default_weight = clamped;
            scene.lowpoly_meshes.insert(*node_id, next);
            let previous_rig_weight = scene
                .character_rig
                .as_mut()
                .map(|rig| rig.morph_weights.insert((*node_id, name.clone()), clamped));
            UndoDelta::RestoreMorphDefaultWeight {
                node_id: *node_id,
                name: name.clone(),
                previous_mesh: current,
                previous_rig_weight,
            }
        }
        Command::ApplyProxyCageDeform {
            node_id,
            divisions,
            control_deltas,
            floor_y,
            bake_as_morph,
        } => {
            let current = scene
                .lowpoly_meshes
                .get(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?
                .clone();
            let mut cage = crate::mesh::ProxyCage::from_mesh_bounds(&current, *divisions, 0.0);
            cage.apply_control_deltas(control_deltas)?;
            let mut next = current.clone();
            if let Some(morph_name) = bake_as_morph {
                let trimmed = morph_name.trim();
                if trimmed.is_empty() {
                    return Err(CoreError::OutOfBounds);
                }
                let baked = cage.bake_to_morph_target(&current, trimmed, *floor_y);
                if let Some(existing) = next.morphs.iter_mut().find(|m| m.name == trimmed) {
                    *existing = baked;
                } else {
                    next.morphs.push(baked);
                }
            } else {
                next.vertices = cage.apply_to_mesh(&current, *floor_y);
            }
            scene.lowpoly_meshes.insert(*node_id, next);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: current,
            }
        }
        Command::BakeProceduralRibbon { params } => {
            let points = crate::mesh::sample_trajectory_points(scene, params)?;
            let mesh = crate::mesh::generate_ribbon_mesh(
                &points,
                params.width_start,
                params.width_end,
                params.material,
            )?;
            let (id, previous_selection) = if params.name.trim().is_empty() {
                let id = scene.allocate_node_id();
                let previous_selection = scene.selected_node;
                scene.nodes.insert(
                    id,
                    SceneNode {
                        id,
                        name: format!("Ribbon_{}", id.0),
                        kind: NodeKind::LowPolyMesh,
                        visible: true,
                        parent: None,
                        transform: Transform::default(),
                    },
                );
                scene.selected_node = Some(id);
                (id, previous_selection)
            } else {
                insert_validated_scene_node(
                    scene,
                    &params.name,
                    NodeKind::LowPolyMesh,
                    None,
                    Transform::default(),
                )?
            };
            scene.lowpoly_meshes.insert(id, mesh);
            scene.selected_node = Some(id);
            if params.bake_visibility_track {
                let previous_rig = scene.character_rig.clone();
                if let Some(rig) = scene.character_rig.as_mut() {
                    if let Some(clip) = rig.animations.get_mut(params.clip_index) {
                        let track = crate::mesh::build_ribbon_visibility_track(
                            id,
                            params.start_time,
                            params.end_time,
                        );
                        clip.visibility_tracks.retain(|t| t.node != id);
                        clip.visibility_tracks.push(track);
                    }
                }
                UndoDelta::Transaction(vec![
                    UndoDelta::RemoveNode {
                        id,
                        previous_selection,
                    },
                    UndoDelta::RestoreCharacterRig(previous_rig),
                ])
            } else {
                UndoDelta::RemoveNode {
                    id,
                    previous_selection,
                }
            }
        }
        Command::CleanupMesh {
            node_id,
            weld_epsilon,
        } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            let previous = mesh.clone();
            crate::mesh::cleanup_mesh(mesh, *weld_epsilon);
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: previous,
            }
        }
        Command::AddMeshModifier { node_id, modifier } => {
            let mesh = scene
                .lowpoly_meshes
                .get_mut(node_id)
                .ok_or(CoreError::LowPolyMeshNotFound(*node_id))?;
            if mesh.modifiers.len() >= 16 {
                return Err(CoreError::OutOfBounds);
            }
            let previous = mesh.clone();
            mesh.modifiers.push(modifier.clone());
            UndoDelta::RestoreLowPolyMesh {
                node_id: *node_id,
                previous_mesh: previous,
            }
        }
        _ => return Err(CoreError::OutOfBounds),
    };
    scene.mark_modified();
    Ok(delta)
}
