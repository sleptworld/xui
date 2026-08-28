use std::sync::Arc;

use crate::{ExternalResourceKind, LayerProgram, ProgramResourceKind};

/// A scene-owned handle bound to one external layer-mask input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerMaskBinding<H> {
    pub ordinal: u32,
    pub handle: H,
}

/// Scene-owned external resources referenced by a [`LayerProgram`].
///
/// Backdrop, parent destination, and layer content are deliberately absent: those
/// resources are supplied by the backend for each plan instantiation. These
/// bindings only carry persistent mask resources owned by the retained scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBindings<H> {
    backdrop_mask: Option<H>,
    layer_masks: Box<[LayerMaskBinding<H>]>,
}

impl<H> ExternalBindings<H> {
    /// Assign dense, deterministic ordinals to layer masks in effect order.
    pub fn new(
        backdrop_mask: Option<H>,
        layer_masks: impl IntoIterator<Item = H>,
    ) -> Result<Self, BindingError> {
        let layer_masks = layer_masks
            .into_iter()
            .enumerate()
            .map(|(index, handle)| {
                Ok(LayerMaskBinding {
                    ordinal: u32::try_from(index).map_err(|_| BindingError::TooManyLayerMasks)?,
                    handle,
                })
            })
            .collect::<Result<Box<[_]>, BindingError>>()?;

        Ok(Self {
            backdrop_mask,
            layer_masks,
        })
    }

    pub const fn backdrop_mask(&self) -> Option<&H> {
        self.backdrop_mask.as_ref()
    }

    pub fn layer_masks(&self) -> &[LayerMaskBinding<H>] {
        &self.layer_masks
    }

    pub fn handle(&self, kind: ExternalResourceKind) -> Option<&H> {
        match kind {
            ExternalResourceKind::BackdropMask => self.backdrop_mask(),
            ExternalResourceKind::LayerMask(ordinal) => self
                .layer_masks
                .get(ordinal as usize)
                .filter(|binding| binding.ordinal == ordinal)
                .map(|binding| &binding.handle),
            ExternalResourceKind::Backdrop
            | ExternalResourceKind::ParentDestination
            | ExternalResourceKind::LayerContent => None,
        }
    }
}

impl<H> Default for ExternalBindings<H> {
    fn default() -> Self {
        Self {
            backdrop_mask: None,
            layer_masks: Box::new([]),
        }
    }
}

/// A reusable static program paired with scene-specific external handles.
///
/// Handles are kept outside the IR and its fingerprint, so resources can be
/// rebound without recompiling the graph.
#[derive(Debug, Clone)]
pub struct BoundLayerProgram<H> {
    program: Arc<LayerProgram>,
    bindings: Arc<ExternalBindings<H>>,
}

impl<H> BoundLayerProgram<H> {
    pub fn new(
        program: Arc<LayerProgram>,
        bindings: ExternalBindings<H>,
    ) -> Result<Self, BindingError> {
        validate_bindings(&program, &bindings)?;
        Ok(Self {
            program,
            bindings: Arc::new(bindings),
        })
    }

    pub const fn program(&self) -> &Arc<LayerProgram> {
        &self.program
    }

    pub fn bindings(&self) -> &ExternalBindings<H> {
        &self.bindings
    }

    pub fn handle(&self, kind: ExternalResourceKind) -> Option<&H> {
        self.bindings.handle(kind)
    }

    pub fn into_parts(self) -> (Arc<LayerProgram>, Arc<ExternalBindings<H>>) {
        (self.program, self.bindings)
    }
}

fn validate_bindings<H>(
    program: &LayerProgram,
    bindings: &ExternalBindings<H>,
) -> Result<(), BindingError> {
    for resource in program.resources() {
        let ProgramResourceKind::External(kind) = resource.kind else {
            continue;
        };
        if matches!(
            kind,
            ExternalResourceKind::BackdropMask | ExternalResourceKind::LayerMask(_)
        ) && bindings.handle(kind).is_none()
        {
            return Err(BindingError::MissingHandle(kind));
        }
    }

    if bindings.backdrop_mask().is_some()
        && program
            .external_resource(ExternalResourceKind::BackdropMask)
            .is_none()
    {
        return Err(BindingError::UnexpectedHandle(
            ExternalResourceKind::BackdropMask,
        ));
    }
    for binding in bindings.layer_masks() {
        let kind = ExternalResourceKind::LayerMask(binding.ordinal);
        if program.external_resource(kind).is_none() {
            return Err(BindingError::UnexpectedHandle(kind));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    #[error("missing handle for external resource {0:?}")]
    MissingHandle(ExternalResourceKind),
    #[error("handle supplied for unused external resource {0:?}")]
    UnexpectedHandle(ExternalResourceKind),
    #[error("the binding contains more than u32::MAX layer masks")]
    TooManyLayerMasks,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile_layer, BackdropDescriptor, LayerEffect, LayerGraphDescriptor, Mask};
    use xui_interface::{Affine, Bounds, ImageData, ImageKey, Rect, Size};

    fn image_mask(key: u64, bounds: Bounds) -> LayerEffect {
        LayerEffect::ImageMask {
            image: ImageKey::UserProvided(key),
            data: ImageData::rgba8(Size::new(1, 1), [255, 255, 255, 255]),
            bounds,
        }
    }

    fn mask_program() -> Arc<LayerProgram> {
        Arc::new(
            compile_layer(&LayerGraphDescriptor {
                backdrop: Some(BackdropDescriptor {
                    mask: Mask::AlphaTexture {
                        texture: ImageKey::UserProvided(1),
                        transform: Affine::IDENTITY,
                    },
                    ..BackdropDescriptor::default()
                }),
                effects: vec![
                    image_mask(2, Bounds::from_origin_size((0.0, 0.0), (1.0, 1.0))),
                    image_mask(3, Bounds::from_origin_size((0.0, 0.0), (1.0, 1.0))),
                ],
                ..LayerGraphDescriptor::default()
            })
            .unwrap(),
        )
    }

    #[test]
    fn binds_masks_by_stable_external_role() {
        let bound = BoundLayerProgram::new(
            mask_program(),
            ExternalBindings::new(Some("backdrop"), ["first", "second"]).unwrap(),
        )
        .unwrap();

        assert_eq!(
            bound.handle(ExternalResourceKind::BackdropMask),
            Some(&"backdrop")
        );
        assert_eq!(
            bound.handle(ExternalResourceKind::LayerMask(0)),
            Some(&"first")
        );
        assert_eq!(
            bound.handle(ExternalResourceKind::LayerMask(1)),
            Some(&"second")
        );
        assert_eq!(bound.handle(ExternalResourceKind::Backdrop), None);
    }

    #[test]
    fn rejects_missing_and_unused_handles() {
        let program = mask_program();
        assert_eq!(
            BoundLayerProgram::new(
                Arc::clone(&program),
                ExternalBindings::new(Some("backdrop"), ["first"]).unwrap(),
            )
            .unwrap_err(),
            BindingError::MissingHandle(ExternalResourceKind::LayerMask(1))
        );

        let plain = Arc::new(compile_layer(&LayerGraphDescriptor::default()).unwrap());
        assert_eq!(
            BoundLayerProgram::new(plain, ExternalBindings::new(Some("unused"), []).unwrap(),)
                .unwrap_err(),
            BindingError::UnexpectedHandle(ExternalResourceKind::BackdropMask)
        );
    }
}
