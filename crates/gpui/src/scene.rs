// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use rustc_hash::FxHashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, EntityId,
    GpuTextureHandle, Hsla, Pixels, Point, Radians, ScaledPixels, Size, bounds_tree::BoundsTree,
    point,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
};

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

#[derive(Debug, Clone)]
pub(crate) struct SceneChunk {
    #[allow(dead_code)]
    pub view_id: EntityId,
    pub shadows: Range<usize>,
    pub quads: Range<usize>,
    pub paths: Range<usize>,
    pub underlines: Range<usize>,
    pub monochrome_sprites: Range<usize>,
    pub subpixel_sprites: Range<usize>,
    pub polychrome_sprites: Range<usize>,
    pub surfaces: Range<usize>,
    pub paint_operations: Range<usize>,
    pub dirty: bool,
}

#[derive(Debug, Default)]
#[expect(missing_docs)]
pub struct ChangedRanges {
    pub shadows: Vec<Range<usize>>,
    pub quads: Vec<Range<usize>>,
    pub paths: Vec<Range<usize>>,
    pub underlines: Vec<Range<usize>>,
    pub monochrome_sprites: Vec<Range<usize>>,
    pub subpixel_sprites: Vec<Range<usize>>,
    pub polychrome_sprites: Vec<Range<usize>>,
    pub surfaces: Vec<Range<usize>>,
}

impl SceneChunk {
    fn new(view_id: EntityId, scene: &Scene) -> Self {
        Self {
            view_id,
            shadows: scene.shadows.len()..scene.shadows.len(),
            quads: scene.quads.len()..scene.quads.len(),
            paths: scene.paths.len()..scene.paths.len(),
            underlines: scene.underlines.len()..scene.underlines.len(),
            monochrome_sprites: scene.monochrome_sprites.len()..scene.monochrome_sprites.len(),
            subpixel_sprites: scene.subpixel_sprites.len()..scene.subpixel_sprites.len(),
            polychrome_sprites: scene.polychrome_sprites.len()..scene.polychrome_sprites.len(),
            surfaces: scene.surfaces.len()..scene.surfaces.len(),
            paint_operations: scene.paint_operations.len()..scene.paint_operations.len(),
            dirty: true,
        }
    }

    fn end_at(&mut self, scene: &Scene) {
        self.shadows.end = scene.shadows.len();
        self.quads.end = scene.quads.len();
        self.paths.end = scene.paths.len();
        self.underlines.end = scene.underlines.len();
        self.monochrome_sprites.end = scene.monochrome_sprites.len();
        self.subpixel_sprites.end = scene.subpixel_sprites.len();
        self.polychrome_sprites.end = scene.polychrome_sprites.len();
        self.surfaces.end = scene.surfaces.len();
        self.paint_operations.end = scene.paint_operations.len();
    }
}

fn merge_ranges(ranges: &mut Vec<Range<usize>>) {
    ranges.retain(|range| range.start < range.end);
    ranges.sort_by_key(|range| range.start);

    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(last_range) = merged.last_mut()
            && range.start <= last_range.end
        {
            last_range.end = last_range.end.max(range.end);
            continue;
        }

        merged.push(range);
    }

    *ranges = merged;
}

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
    damage_rects: Vec<Bounds<ScaledPixels>>,
    pub(crate) chunks: Vec<SceneChunk>,
    pub(crate) active_chunk: Option<usize>,
    pub(crate) chunk_map: FxHashMap<EntityId, usize>,
    pub(crate) generation: u64,
    batch_count: u32,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
        self.damage_rects.clear();
        self.chunks.clear();
        self.active_chunk = None;
        self.chunk_map.clear();
        self.batch_count = 0;
    }

    pub fn begin_view_chunk(&mut self, view_id: EntityId) {
        let chunk = SceneChunk::new(view_id, self);
        let chunk_index = if let Some(&chunk_index) = self.chunk_map.get(&view_id) {
            if let Some(existing_chunk) = self.chunks.get_mut(chunk_index) {
                *existing_chunk = chunk;
            }
            chunk_index
        } else {
            let chunk_index = self.chunks.len();
            self.chunks.push(chunk);
            self.chunk_map.insert(view_id, chunk_index);
            chunk_index
        };

        self.active_chunk = Some(chunk_index);
        self.generation += 1;
    }

    pub fn end_view_chunk(&mut self) {
        if let Some(chunk_index) = self.active_chunk.take() {
            let mut chunk = if let Some(chunk) = self.chunks.get(chunk_index) {
                chunk.clone()
            } else {
                return;
            };
            chunk.end_at(self);
            if let Some(existing_chunk) = self.chunks.get_mut(chunk_index) {
                *existing_chunk = chunk;
            }
        }
    }

    pub fn mark_chunk_clean(&mut self, view_id: EntityId) {
        if let Some(&chunk_index) = self.chunk_map.get(&view_id)
            && let Some(chunk) = self.chunks.get_mut(chunk_index)
        {
            chunk.dirty = false;
        }
    }

    pub fn is_chunk_dirty(&self, view_id: EntityId) -> bool {
        self.chunk_map
            .get(&view_id)
            .and_then(|&chunk_index| self.chunks.get(chunk_index))
            .map_or(true, |chunk| chunk.dirty)
    }

    pub fn scene_generation(&self) -> u64 {
        self.generation
    }

    pub fn dirty_chunk_count(&self) -> usize {
        self.chunks.iter().filter(|chunk| chunk.dirty).count()
    }

    pub fn batch_count(&self) -> u32 {
        self.batch_count
    }

    pub fn has_changes(&self) -> bool {
        self.chunks.iter().any(|chunk| chunk.dirty)
    }

    pub fn changed_ranges(&self) -> ChangedRanges {
        let mut ranges = ChangedRanges::default();

        for chunk in self.chunks.iter().filter(|chunk| chunk.dirty) {
            ranges.shadows.push(chunk.shadows.clone());
            ranges.quads.push(chunk.quads.clone());
            ranges.paths.push(chunk.paths.clone());
            ranges.underlines.push(chunk.underlines.clone());
            ranges
                .monochrome_sprites
                .push(chunk.monochrome_sprites.clone());
            ranges.subpixel_sprites.push(chunk.subpixel_sprites.clone());
            ranges
                .polychrome_sprites
                .push(chunk.polychrome_sprites.clone());
            ranges.surfaces.push(chunk.surfaces.clone());
        }

        merge_ranges(&mut ranges.shadows);
        merge_ranges(&mut ranges.quads);
        merge_ranges(&mut ranges.paths);
        merge_ranges(&mut ranges.underlines);
        merge_ranges(&mut ranges.monochrome_sprites);
        merge_ranges(&mut ranges.subpixel_sprites);
        merge_ranges(&mut ranges.polychrome_sprites);
        merge_ranges(&mut ranges.surfaces);

        ranges
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
            }
        }
    }

    pub fn finish(&mut self) {
        let dirty_before = self.dirty_chunk_count();
        let chunks_before = self.chunks.len();
        self.finish_incremental();
        self.compute_damage_rects();
        tracing::debug!(
            target: "gpui::scene",
            chunks_before,
            dirty_before,
            chunks_after = self.chunks.len(),
            damage_rects = self.damage_rects.len(),
            damage_area = self.damage_area(),
            shadows = self.shadows.len(),
            quads = self.quads.len(),
            paths = self.paths.len(),
            underlines = self.underlines.len(),
            monochrome_sprites = self.monochrome_sprites.len(),
            polychrome_sprites = self.polychrome_sprites.len(),
            surfaces = self.surfaces.len(),
            "scene finished"
        );
        for chunk in &mut self.chunks {
            chunk.dirty = false;
        }
    }

    fn finish_incremental(&mut self) {
        let total_chunk_count = self.chunks.len();
        let dirty_chunk_count = self.dirty_chunk_count();

        if total_chunk_count == 0 || dirty_chunk_count * 10 >= total_chunk_count * 3 {
            self.sort_all_primitives();
        } else {
            for chunk in &mut self.chunks {
                if !chunk.dirty {
                    continue;
                }

                if let Some(shadows) = self.shadows.get_mut(chunk.shadows.clone()) {
                    shadows.sort_by_key(|shadow| (shadow.order, PrimitiveKind::Shadow));
                }
                if let Some(quads) = self.quads.get_mut(chunk.quads.clone()) {
                    quads.sort_by_key(|quad| (quad.order, PrimitiveKind::Quad));
                }
                if let Some(paths) = self.paths.get_mut(chunk.paths.clone()) {
                    paths.sort_by_key(|path| (path.order, PrimitiveKind::Path));
                }
                if let Some(underlines) = self.underlines.get_mut(chunk.underlines.clone()) {
                    underlines.sort_by_key(|underline| (underline.order, PrimitiveKind::Underline));
                }
                if let Some(monochrome_sprites) = self
                    .monochrome_sprites
                    .get_mut(chunk.monochrome_sprites.clone())
                {
                    monochrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
                }
                if let Some(subpixel_sprites) = self
                    .subpixel_sprites
                    .get_mut(chunk.subpixel_sprites.clone())
                {
                    subpixel_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
                }
                if let Some(polychrome_sprites) = self
                    .polychrome_sprites
                    .get_mut(chunk.polychrome_sprites.clone())
                {
                    polychrome_sprites.sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
                }
                if let Some(surfaces) = self.surfaces.get_mut(chunk.surfaces.clone()) {
                    surfaces.sort_by_key(|surface| (surface.order, PrimitiveKind::Surface));
                }
            }
        }

        self.batch_count = self.total_batch_count();
        self.generation += 1;
    }

    pub fn compute_damage_rects(&mut self) {
        self.damage_rects.clear();

        for chunk in &self.chunks {
            if !chunk.dirty {
                continue;
            }

            if let Some(bounds) = self.chunk_bounds(chunk) {
                self.damage_rects.push(bounds);
            }
        }

        self.merge_damage_rects();
    }

    pub fn damage_rects(&self) -> &[Bounds<ScaledPixels>] {
        &self.damage_rects
    }

    pub fn damage_area(&self) -> f32 {
        self.damage_rects
            .iter()
            .map(|bounds| bounds.size.width.as_f32() * bounds.size.height.as_f32())
            .sum()
    }

    pub fn full_redraw_needed(&self, viewport: Bounds<ScaledPixels>) -> bool {
        if self.chunks.is_empty() {
            return true;
        }

        let viewport_area = viewport.size.width.as_f32() * viewport.size.height.as_f32();
        self.damage_area() > viewport_area * 0.7
    }

    fn chunk_bounds(&self, chunk: &SceneChunk) -> Option<Bounds<ScaledPixels>> {
        let mut bounds = None;

        if let Some(shadows) = self.shadows.get(chunk.shadows.clone()) {
            for shadow in shadows {
                Self::include_bounds(&mut bounds, shadow.bounds);
            }
        }
        if let Some(quads) = self.quads.get(chunk.quads.clone()) {
            for quad in quads {
                Self::include_bounds(&mut bounds, quad.bounds);
            }
        }
        if let Some(paths) = self.paths.get(chunk.paths.clone()) {
            for path in paths {
                Self::include_bounds(&mut bounds, path.bounds);
            }
        }
        if let Some(underlines) = self.underlines.get(chunk.underlines.clone()) {
            for underline in underlines {
                Self::include_bounds(&mut bounds, underline.bounds);
            }
        }
        if let Some(monochrome_sprites) = self
            .monochrome_sprites
            .get(chunk.monochrome_sprites.clone())
        {
            for sprite in monochrome_sprites {
                Self::include_bounds(&mut bounds, sprite.bounds);
            }
        }
        if let Some(subpixel_sprites) = self.subpixel_sprites.get(chunk.subpixel_sprites.clone()) {
            for sprite in subpixel_sprites {
                Self::include_bounds(&mut bounds, sprite.bounds);
            }
        }
        if let Some(polychrome_sprites) = self
            .polychrome_sprites
            .get(chunk.polychrome_sprites.clone())
        {
            for sprite in polychrome_sprites {
                Self::include_bounds(&mut bounds, sprite.bounds);
            }
        }
        if let Some(surfaces) = self.surfaces.get(chunk.surfaces.clone()) {
            for surface in surfaces {
                Self::include_bounds(&mut bounds, surface.bounds);
            }
        }

        bounds
    }

    fn include_bounds(
        bounds: &mut Option<Bounds<ScaledPixels>>,
        primitive_bounds: Bounds<ScaledPixels>,
    ) {
        *bounds = Some(bounds.map_or(primitive_bounds, |bounds| bounds.union(&primitive_bounds)));
    }

    fn merge_damage_rects(&mut self) {
        let mut merged = Vec::new();

        for mut bounds in self.damage_rects.drain(..) {
            let mut index = 0;
            while index < merged.len() {
                if bounds.intersects(&merged[index]) {
                    let existing_bounds = merged.remove(index);
                    bounds = bounds.union(&existing_bounds);
                    index = 0;
                } else {
                    index += 1;
                }
            }
            merged.push(bounds);
        }

        self.damage_rects = merged;
    }

    fn sort_all_primitives(&mut self) {
        self.shadows
            .sort_by_key(|shadow| (shadow.order, PrimitiveKind::Shadow));
        self.quads
            .sort_by_key(|quad| (quad.order, PrimitiveKind::Quad));
        self.paths
            .sort_by_key(|path| (path.order, PrimitiveKind::Path));
        self.underlines
            .sort_by_key(|underline| (underline.order, PrimitiveKind::Underline));
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces
            .sort_by_key(|surface| (surface.order, PrimitiveKind::Surface));
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }

    pub fn total_batch_count(&self) -> u32 {
        let mut counter = BatchCounter::default();
        let mut count = 0u32;
        while counter.advance(self) {
            count = count.saturating_add(1);
        }
        count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }
}

#[derive(Default)]
struct BatchCounter {
    shadows_index: usize,
    quads_index: usize,
    paths_index: usize,
    underlines_index: usize,
    monochrome_sprites_index: usize,
    subpixel_sprites_index: usize,
    polychrome_sprites_index: usize,
    surfaces_index: usize,
}

impl BatchCounter {
    fn advance(&mut self, scene: &Scene) -> bool {
        let mut orders_and_kinds = [
            (
                scene
                    .shadows
                    .get(self.shadows_index)
                    .map(|shadow| shadow.order),
                PrimitiveKind::Shadow,
            ),
            (
                scene.quads.get(self.quads_index).map(|quad| quad.order),
                PrimitiveKind::Quad,
            ),
            (
                scene.paths.get(self.paths_index).map(|path| path.order),
                PrimitiveKind::Path,
            ),
            (
                scene
                    .underlines
                    .get(self.underlines_index)
                    .map(|underline| underline.order),
                PrimitiveKind::Underline,
            ),
            (
                scene
                    .monochrome_sprites
                    .get(self.monochrome_sprites_index)
                    .map(|sprite| sprite.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                scene
                    .subpixel_sprites
                    .get(self.subpixel_sprites_index)
                    .map(|sprite| sprite.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                scene
                    .polychrome_sprites
                    .get(self.polychrome_sprites_index)
                    .map(|sprite| sprite.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                scene
                    .surfaces
                    .get(self.surfaces_index)
                    .map(|surface| surface.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let Some(_) = first.0 else {
            return false;
        };

        let batch_kind = first.1;
        let max_order_and_kind = (second.0.unwrap_or(u32::MAX), second.1);

        match batch_kind {
            PrimitiveKind::Shadow => {
                self.shadows_index += 1;
                while let Some(shadow) = scene.shadows.get(self.shadows_index)
                    && (shadow.order, batch_kind) < max_order_and_kind
                {
                    self.shadows_index += 1;
                }
            }
            PrimitiveKind::Quad => {
                self.quads_index += 1;
                while let Some(quad) = scene.quads.get(self.quads_index)
                    && (quad.order, batch_kind) < max_order_and_kind
                {
                    self.quads_index += 1;
                }
            }
            PrimitiveKind::Path => {
                self.paths_index += 1;
                while let Some(path) = scene.paths.get(self.paths_index)
                    && (path.order, batch_kind) < max_order_and_kind
                {
                    self.paths_index += 1;
                }
            }
            PrimitiveKind::Underline => {
                self.underlines_index += 1;
                while let Some(underline) = scene.underlines.get(self.underlines_index)
                    && (underline.order, batch_kind) < max_order_and_kind
                {
                    self.underlines_index += 1;
                }
            }
            PrimitiveKind::MonochromeSprite => {
                let Some(texture_id) = scene
                    .monochrome_sprites
                    .get(self.monochrome_sprites_index)
                    .map(|sprite| sprite.tile.texture_id)
                else {
                    return false;
                };
                self.monochrome_sprites_index += 1;
                while let Some(sprite) = scene.monochrome_sprites.get(self.monochrome_sprites_index)
                    && (sprite.order, batch_kind) < max_order_and_kind
                    && sprite.tile.texture_id == texture_id
                {
                    self.monochrome_sprites_index += 1;
                }
            }
            PrimitiveKind::SubpixelSprite => {
                let Some(texture_id) = scene
                    .subpixel_sprites
                    .get(self.subpixel_sprites_index)
                    .map(|sprite| sprite.tile.texture_id)
                else {
                    return false;
                };
                self.subpixel_sprites_index += 1;
                while let Some(sprite) = scene.subpixel_sprites.get(self.subpixel_sprites_index)
                    && (sprite.order, batch_kind) < max_order_and_kind
                    && sprite.tile.texture_id == texture_id
                {
                    self.subpixel_sprites_index += 1;
                }
            }
            PrimitiveKind::PolychromeSprite => {
                let Some(texture_id) = scene
                    .polychrome_sprites
                    .get(self.polychrome_sprites_index)
                    .map(|sprite| sprite.tile.texture_id)
                else {
                    return false;
                };
                self.polychrome_sprites_index += 1;
                while let Some(sprite) = scene.polychrome_sprites.get(self.polychrome_sprites_index)
                    && (sprite.order, batch_kind) < max_order_and_kind
                    && sprite.tile.texture_id == texture_id
                {
                    self.polychrome_sprites_index += 1;
                }
            }
            PrimitiveKind::Surface => {
                self.surfaces_index += 1;
                while let Some(surface) = scene.surfaces.get(self.surfaces_index)
                    && (surface.order, batch_kind) < max_order_and_kind
                {
                    self.surfaces_index += 1;
                }
            }
        }

        true
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let Some(sprite) = self.monochrome_sprites_iter.peek() else {
                    return None;
                };
                let texture_id = sprite.tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let Some(sprite) = self.subpixel_sprites_iter.peek() else {
                    return None;
                };
                let texture_id = sprite.tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let Some(sprite) = self.polychrome_sprites_iter.peek() else {
                    return None;
                };
                let texture_id = sprite.tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: u32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: bool,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub content: SurfaceContent,
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub enum SurfaceContent {
    #[cfg(target_os = "macos")]
    CoreVideo(core_video::pixel_buffer::CVPixelBuffer),
    ExternalTexture(GpuTextureHandle),
}

impl SurfaceContent {
    /// Return the external GPU texture when this surface was created from one.
    pub fn external_texture(&self) -> Option<&GpuTextureHandle> {
        #[cfg(target_os = "macos")]
        {
            match self {
                Self::CoreVideo(_) => None,
                Self::ExternalTexture(texture) => Some(texture),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let Self::ExternalTexture(texture) = self;
            Some(texture)
        }
    }
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}
