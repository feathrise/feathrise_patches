use crate::{
    render_asset::{AssetExtractionError, PrepareAssetError, RenderAsset},
    render_resource::{DefaultImageSampler, Sampler, Texture, TextureView},
    renderer::{RenderDevice, RenderQueue},
};
use bevy_asset::{AssetId, RenderAssetUsages};
use bevy_ecs::system::{lifetimeless::SRes, SystemParamItem};
use bevy_image::{Image, ImageSampler};
use bevy_log::warn;
use bevy_math::{AspectRatio, UVec2};
use wgpu::{Extent3d, TexelCopyBufferLayout, TextureFormat, TextureUsages};
use wgpu_types::{TextureDescriptor, TextureViewDescriptor};

/// The GPU-representation of an [`Image`].
/// Consists of the [`Texture`], its [`TextureView`] and the corresponding [`Sampler`], and the texture's size.
#[derive(Debug, Clone)]
pub struct GpuImage {
    pub texture: Texture,
    pub texture_view: TextureView,
    pub sampler: Sampler,
    pub texture_descriptor: TextureDescriptor<Option<&'static str>, &'static [TextureFormat]>,
    pub texture_view_descriptor: Option<TextureViewDescriptor<Option<&'static str>>>,
    pub had_data: bool,
}

impl RenderAsset for GpuImage {
    type SourceAsset = Image;
    type Param = (
        SRes<RenderDevice>,
        SRes<RenderQueue>,
        SRes<DefaultImageSampler>,
    );

    #[inline]
    fn asset_usage(image: &Self::SourceAsset) -> RenderAssetUsages {
        image.asset_usage
    }

    fn take_gpu_data(
        source: &mut Self::SourceAsset,
        previous_gpu_asset: Option<&Self>,
    ) -> Result<Self::SourceAsset, AssetExtractionError> {
        let data = source.data.take();

        // check if this image originally had data and no longer does, that implies it
        // has already been extracted
        let valid_upload = data.is_some() || previous_gpu_asset.is_none_or(|prev| !prev.had_data);

        valid_upload
            .then(|| Self::SourceAsset {
                data,
                ..source.clone()
            })
            .ok_or(AssetExtractionError::AlreadyExtracted)
    }

    #[inline]
    fn byte_len(image: &Self::SourceAsset) -> Option<usize> {
        image.data.as_ref().map(Vec::len)
    }

    /// Converts the extracted image into a [`GpuImage`].
    fn prepare_asset(
        image: Self::SourceAsset,
        _: AssetId<Self::SourceAsset>,
        (render_device, render_queue, default_sampler): &mut SystemParamItem<Self::Param>,
        previous_asset: Option<&Self>,
    ) -> Result<Self, PrepareAssetError<Self::SourceAsset>> {
        let had_data = image.data.is_some();
        let texture = if let Some(prev) = previous_asset
            && prev.texture_descriptor == image.texture_descriptor
            && (!had_data
                || prev
                    .texture_descriptor
                    .usage
                    .contains(TextureUsages::COPY_DST))
            && let Some(block_bytes) = image.texture_descriptor.format.block_copy_size(None)
        {
            if let Some(ref data) = image.data {
                let (block_width, block_height) =
                    image.texture_descriptor.format.block_dimensions();
                // 热更新必须覆盖完整 Mip/数组层，且与 create_texture_with_data 的字节顺序一致。
                // 只写 mip 0 会让远处继续采样旧空白权重；保留原纹理与视图，已有绑定同步看到新数据。
                let descriptor = &image.texture_descriptor;
                let layers = descriptor.array_layer_count();
                let (outer_count, inner_count) = match image.data_order {
                    wgpu::util::TextureDataOrder::LayerMajor => (layers, descriptor.mip_level_count),
                    wgpu::util::TextureDataOrder::MipMajor => (descriptor.mip_level_count, layers),
                };
                let mut offset = 0;
                for outer in 0..outer_count {
                    for inner in 0..inner_count {
                        let (layer, mip) = match image.data_order {
                            wgpu::util::TextureDataOrder::LayerMajor => (outer, inner),
                            wgpu::util::TextureDataOrder::MipMajor => (inner, outer),
                        };
                        let mut size = descriptor.mip_level_size(mip).unwrap();
                        if descriptor.dimension != wgpu::TextureDimension::D3 {
                            size.depth_or_array_layers = 1;
                        }
                        // BC 等压缩纹理的尾 Mip 仍按完整压缩块上传；三维纹理每级包含全部深度切片。
                        let physical = size.physical_size(descriptor.format);
                        let bytes_per_row = physical.width / block_width * block_bytes;
                        let rows_per_image = physical.height / block_height;
                        let end = offset
                            + (bytes_per_row * rows_per_image * size.depth_or_array_layers) as usize;
                        render_queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &prev.texture,
                                mip_level: mip,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: layer,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &data[offset..end],
                            TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(bytes_per_row),
                                rows_per_image: Some(rows_per_image),
                            },
                            physical,
                        );
                        offset = end;
                    }
                }
            }

            if !image.copy_on_resize {
                // TODO else could clear here? probably not necessary as textures without data are only useful as render
                // targets and will normally be overwritten immediately anyway
            }

            // reuse previous texture
            prev.texture.clone()
        } else if let Some(ref data) = image.data {
            render_device.create_texture_with_data(
                render_queue,
                &image.texture_descriptor,
                image.data_order,
                data,
            )
        } else {
            let new_texture = render_device.create_texture(&image.texture_descriptor);
            if image.copy_on_resize {
                if let Some(previous) = previous_asset {
                    let mut command_encoder =
                        render_device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("copy_image_on_resize"),
                        });
                    let copy_size = Extent3d {
                        width: image
                            .texture_descriptor
                            .size
                            .width
                            .min(previous.texture_descriptor.size.width),
                        height: image
                            .texture_descriptor
                            .size
                            .height
                            .min(previous.texture_descriptor.size.height),
                        depth_or_array_layers: image
                            .texture_descriptor
                            .size
                            .depth_or_array_layers
                            .min(previous.texture_descriptor.size.depth_or_array_layers),
                    };

                    command_encoder.copy_texture_to_texture(
                        previous.texture.as_image_copy(),
                        new_texture.as_image_copy(),
                        copy_size,
                    );
                    render_queue.submit([command_encoder.finish()]);
                } else {
                    warn!("No previous asset to copy from for image: {:?}", image);
                }
            }
            new_texture
        };

        let texture_view = if let Some(prev) = previous_asset.as_ref()
            && prev.texture_descriptor == image.texture_descriptor
            && prev
                .texture_descriptor
                .usage
                .contains(TextureUsages::COPY_DST)
            && prev.texture_view_descriptor == image.texture_view_descriptor
        {
            prev.texture_view.clone()
        } else {
            image
                .texture_view_descriptor
                .as_ref()
                .map(|desc| texture.create_view(desc))
                .unwrap_or_else(|| texture.create_view(&TextureViewDescriptor::default()))
        };
        let sampler = match image.sampler {
            ImageSampler::Default => (***default_sampler).clone(),
            ImageSampler::Descriptor(descriptor) => {
                render_device.create_sampler(&descriptor.as_wgpu())
            }
        };

        Ok(GpuImage {
            texture,
            texture_view,
            sampler,
            texture_descriptor: image.texture_descriptor,
            texture_view_descriptor: image.texture_view_descriptor,
            had_data,
        })
    }
}

impl GpuImage {
    /// Returns the aspect ratio (width / height) of a 2D image.
    #[inline]
    pub fn aspect_ratio(&self) -> AspectRatio {
        AspectRatio::try_from_pixels(
            self.texture_descriptor.size.width,
            self.texture_descriptor.size.height,
        )
        .expect(
            "Failed to calculate aspect ratio: Image dimensions must be positive, non-zero values",
        )
    }

    /// Returns the size of a 2D image.
    #[inline]
    pub fn size_2d(&self) -> UVec2 {
        UVec2::new(
            self.texture_descriptor.size.width,
            self.texture_descriptor.size.height,
        )
    }

    /// Gets the view format of this image.
    /// If the view format is not explicitly provided, falls back to the base image format
    #[inline]
    pub fn view_format(&self) -> TextureFormat {
        self.texture_view_descriptor
            .as_ref()
            .and_then(|view_desc| view_desc.format)
            .unwrap_or(self.texture_descriptor.format)
    }
}
