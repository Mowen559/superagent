use rapid_ocr_rs::Quad;

const CHANNELS: usize = 4;
const BACKGROUND_BLUR_SIGMA: f32 = 8.0;
const GAUSSIAN_BOX_PASSES: usize = 3;

pub fn white_blur_fill(
    rgba: &mut [u8],
    width: usize,
    height: usize,
    line_quads: &[Quad],
) -> Vec<[u8; 4]> {
    let pixel_count = width.saturating_mul(height);
    if rgba.len() != pixel_count.saturating_mul(CHANNELS) || pixel_count == 0 {
        return vec![[0, 0, 0, 255]; line_quads.len()];
    }

    let mut global_mask = vec![false; pixel_count];
    for line_quad in line_quads {
        let short_side = quad_short_side(line_quad).max(1.0);
        let margin = (short_side * 0.08).clamp(1.0, 4.0);
        rasterize_quad(
            &expanded_quad(line_quad, margin),
            width,
            height,
            &mut global_mask,
        );
    }

    let mut foregrounds = Vec::with_capacity(line_quads.len());
    let mut line_mask = vec![false; pixel_count];
    for line_quad in line_quads {
        let short_side = quad_short_side(line_quad).max(1.0);
        let margin = (short_side * 0.08).clamp(1.0, 4.0);
        line_mask.fill(false);
        rasterize_quad(
            &expanded_quad(line_quad, margin),
            width,
            height,
            &mut line_mask,
        );
        let ring = exterior_ring(line_quad, &line_mask, &global_mask, width, height);
        let background = robust_color(rgba, &ring).unwrap_or([255, 255, 255, 255]);
        foregrounds.push(estimate_foreground(rgba, &line_mask, background));
    }

    let blurred = gaussian_blur(rgba, width, height, BACKGROUND_BLUR_SIGMA);
    for (index, masked) in global_mask.into_iter().enumerate() {
        if !masked {
            continue;
        }
        let offset = index * CHANNELS;
        for channel in 0..3 {
            rgba[offset + channel] = ((blurred[offset + channel] as u16 + 256) / 2) as u8;
        }
        rgba[offset + 3] = 255;
    }
    foregrounds
}

fn gaussian_blur(source: &[u8], width: usize, height: usize, sigma: f32) -> Vec<u8> {
    let mut blurred = source.to_vec();
    for radius in gaussian_box_radii(sigma) {
        blurred = vertical_box_blur(
            &horizontal_box_blur(&blurred, width, height, radius),
            width,
            height,
            radius,
        );
    }
    blurred
}

fn gaussian_box_radii(sigma: f32) -> [usize; GAUSSIAN_BOX_PASSES] {
    let ideal_width = ((12.0 * sigma * sigma / GAUSSIAN_BOX_PASSES as f32) + 1.0).sqrt();
    let mut lower_width = ideal_width.floor() as i32;
    if lower_width % 2 == 0 {
        lower_width -= 1;
    }
    let upper_width = lower_width + 2;
    let lower_passes = (12.0 * sigma * sigma
        - GAUSSIAN_BOX_PASSES as f32 * lower_width.pow(2) as f32
        - 4.0 * GAUSSIAN_BOX_PASSES as f32 * lower_width as f32
        - 3.0 * GAUSSIAN_BOX_PASSES as f32)
        / (-4.0 * lower_width as f32 - 4.0);
    let lower_pass_count =
        (lower_passes.round() as i32).clamp(0, GAUSSIAN_BOX_PASSES as i32) as usize;

    std::array::from_fn(|pass| {
        let width = if pass < lower_pass_count {
            lower_width
        } else {
            upper_width
        };
        ((width - 1) / 2) as usize
    })
}

fn horizontal_box_blur(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || source.is_empty() {
        return source.to_vec();
    }

    let mut output = vec![0_u8; source.len()];
    let kernel_width = radius * 2 + 1;
    for y in 0..height {
        let mut sums = [0_usize; CHANNELS];
        for offset in -(radius as isize)..=radius as isize {
            let source_x = offset.clamp(0, width.saturating_sub(1) as isize) as usize;
            let source_offset = (y * width + source_x) * CHANNELS;
            for channel in 0..CHANNELS {
                sums[channel] += source[source_offset + channel] as usize;
            }
        }

        for x in 0..width {
            let output_offset = (y * width + x) * CHANNELS;
            for channel in 0..CHANNELS {
                output[output_offset + channel] =
                    ((sums[channel] + kernel_width / 2) / kernel_width) as u8;
            }

            let removed_x = x.saturating_sub(radius);
            let added_x = (x + radius + 1).min(width - 1);
            let removed_offset = (y * width + removed_x) * CHANNELS;
            let added_offset = (y * width + added_x) * CHANNELS;
            for channel in 0..CHANNELS {
                sums[channel] += source[added_offset + channel] as usize;
                sums[channel] -= source[removed_offset + channel] as usize;
            }
        }
    }
    output
}

fn vertical_box_blur(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || source.is_empty() {
        return source.to_vec();
    }

    let mut output = vec![0_u8; source.len()];
    let kernel_width = radius * 2 + 1;
    for x in 0..width {
        let mut sums = [0_usize; CHANNELS];
        for offset in -(radius as isize)..=radius as isize {
            let source_y = offset.clamp(0, height.saturating_sub(1) as isize) as usize;
            let source_offset = (source_y * width + x) * CHANNELS;
            for channel in 0..CHANNELS {
                sums[channel] += source[source_offset + channel] as usize;
            }
        }

        for y in 0..height {
            let output_offset = (y * width + x) * CHANNELS;
            for channel in 0..CHANNELS {
                output[output_offset + channel] =
                    ((sums[channel] + kernel_width / 2) / kernel_width) as u8;
            }

            let removed_y = y.saturating_sub(radius);
            let added_y = (y + radius + 1).min(height - 1);
            let removed_offset = (removed_y * width + x) * CHANNELS;
            let added_offset = (added_y * width + x) * CHANNELS;
            for channel in 0..CHANNELS {
                sums[channel] += source[added_offset + channel] as usize;
                sums[channel] -= source[removed_offset + channel] as usize;
            }
        }
    }
    output
}

fn expanded_quad(quad: &Quad, margin: f32) -> Quad {
    let center = quad.iter().fold([0.0_f32; 2], |mut center, point| {
        center[0] += point[0] * 0.25;
        center[1] += point[1] * 0.25;
        center
    });
    let mut out = *quad;
    for (dst, src) in out.iter_mut().zip(quad) {
        let dx = src[0] - center[0];
        let dy = src[1] - center[1];
        let length = (dx * dx + dy * dy).sqrt().max(1.0);
        dst[0] += dx * margin / length;
        dst[1] += dy * margin / length;
    }
    out
}

fn quad_short_side(quad: &Quad) -> f32 {
    let distance = |a: [f32; 2], b: [f32; 2]| {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        (dx * dx + dy * dy).sqrt()
    };
    distance(quad[0], quad[3]).min(distance(quad[1], quad[2]))
}

fn quad_bounds(
    quad: &Quad,
    width: usize,
    height: usize,
    margin: f32,
) -> (usize, usize, usize, usize) {
    let min_x = quad.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - margin;
    let max_x = quad.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max) + margin;
    let min_y = quad.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min) - margin;
    let max_y = quad.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max) + margin;
    (
        min_x.floor().max(0.0) as usize,
        min_y.floor().max(0.0) as usize,
        max_x.ceil().clamp(0.0, width.saturating_sub(1) as f32) as usize,
        max_y.ceil().clamp(0.0, height.saturating_sub(1) as f32) as usize,
    )
}

fn rasterize_quad(quad: &Quad, width: usize, height: usize, mask: &mut [bool]) {
    let (min_x, min_y, max_x, max_y) = quad_bounds(quad, width, height, 0.0);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if point_in_quad(x as f32 + 0.5, y as f32 + 0.5, quad) {
                mask[y * width + x] = true;
            }
        }
    }
}

fn point_in_quad(x: f32, y: f32, quad: &Quad) -> bool {
    let mut sign = 0_i8;
    for edge in 0..4 {
        let a = quad[edge];
        let b = quad[(edge + 1) % 4];
        let cross = (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        if cross.abs() < 0.001 {
            continue;
        }
        let current = if cross > 0.0 { 1 } else { -1 };
        if sign != 0 && current != sign {
            return false;
        }
        sign = current;
    }
    true
}

fn exterior_ring(
    quad: &Quad,
    line_mask: &[bool],
    global_mask: &[bool],
    width: usize,
    height: usize,
) -> Vec<usize> {
    let radius = (quad_short_side(quad) * 0.45).clamp(3.0, 18.0);
    let (min_x, min_y, max_x, max_y) = quad_bounds(quad, width, height, radius);
    let mut ring = Vec::new();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let index = y * width + x;
            if !line_mask[index] && !global_mask[index] {
                ring.push(index);
            }
        }
    }
    ring
}

fn robust_color(rgba: &[u8], indices: &[usize]) -> Option<[u8; 4]> {
    if indices.is_empty() {
        return None;
    }
    let mut channels = [Vec::with_capacity(indices.len()), Vec::new(), Vec::new()];
    channels[1].reserve(indices.len());
    channels[2].reserve(indices.len());
    for &index in indices {
        let offset = index * CHANNELS;
        channels[0].push(rgba[offset]);
        channels[1].push(rgba[offset + 1]);
        channels[2].push(rgba[offset + 2]);
    }
    for channel in &mut channels {
        channel.sort_unstable();
    }
    let middle = indices.len() / 2;
    Some([
        channels[0][middle],
        channels[1][middle],
        channels[2][middle],
        255,
    ])
}

fn estimate_foreground(rgba: &[u8], mask: &[bool], background: [u8; 4]) -> [u8; 4] {
    let mut candidates = mask
        .iter()
        .enumerate()
        .filter(|(_, masked)| **masked)
        .map(|(index, _)| {
            let offset = index * CHANNELS;
            let color = [rgba[offset], rgba[offset + 1], rgba[offset + 2]];
            let distance = color
                .iter()
                .zip(background)
                .map(|(value, bg)| (*value as f32 - bg as f32).powi(2))
                .sum::<f32>();
            (distance, color)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return [0, 0, 0, 255];
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let keep = (candidates.len() / 3).max(1);
    let mut values = [
        Vec::with_capacity(keep),
        Vec::with_capacity(keep),
        Vec::with_capacity(keep),
    ];
    for (_, color) in candidates.into_iter().take(keep) {
        for channel in 0..3 {
            values[channel].push(color[channel]);
        }
    }
    for value in &mut values {
        value.sort_unstable();
    }
    [
        values[0][values[0].len() / 2],
        values[1][values[1].len() / 2],
        values[2][values[2].len() / 2],
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_blur_fill_preserves_pixels_outside_mask() {
        let width = 24;
        let height = 12;
        let mut image = vec![0_u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                image[offset] = 80 + x as u8;
                image[offset + 1] = 100 + y as u8;
                image[offset + 2] = 140;
                image[offset + 3] = 255;
            }
        }
        for y in 4..8 {
            for x in 8..16 {
                let offset = (y * width + x) * 4;
                image[offset..offset + 3].fill(10);
            }
        }
        let original = image.clone();
        let quad = [[8.0, 4.0], [16.0, 4.0], [16.0, 8.0], [8.0, 8.0]];
        white_blur_fill(&mut image, width, height, &[quad]);
        assert_eq!(&image[..4 * width * 2], &original[..4 * width * 2]);
        assert_ne!(image[(5 * width + 10) * 4], 10);
    }

    #[test]
    fn white_blur_fill_mixes_masked_pixels_equally_with_white() {
        let width = 12;
        let height = 8;
        let mut image = vec![0_u8; width * height * CHANNELS];
        for pixel in image.chunks_exact_mut(CHANNELS) {
            pixel.copy_from_slice(&[0, 64, 255, 80]);
        }
        let original = image.clone();
        let quad = [[3.0, 2.0], [9.0, 2.0], [9.0, 6.0], [3.0, 6.0]];

        white_blur_fill(&mut image, width, height, &[quad]);

        assert_eq!(&image[..CHANNELS], &original[..CHANNELS]);
        let center = (4 * width + 6) * CHANNELS;
        assert_eq!(&image[center..center + CHANNELS], &[128, 160, 255, 255]);
    }

    #[test]
    fn gaussian_blur_softens_a_hard_edge() {
        let width = 96;
        let height = 24;
        let mut image = vec![0_u8; width * height * CHANNELS];
        for y in 0..height {
            for x in width / 2..width {
                let offset = (y * width + x) * CHANNELS;
                image[offset..offset + CHANNELS].fill(255);
            }
        }

        let blurred = gaussian_blur(&image, width, height, BACKGROUND_BLUR_SIGMA);
        let left = (12 * width) * CHANNELS;
        let transition = (12 * width + width / 2 - 1) * CHANNELS;
        let right = (12 * width + width - 1) * CHANNELS;
        assert_eq!(blurred[left], 0);
        assert!(blurred[transition] > 0 && blurred[transition] < 255);
        assert_eq!(blurred[right], 255);
    }
}
