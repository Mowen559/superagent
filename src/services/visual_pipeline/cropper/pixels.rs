//! Top-down, physical-pixel operations shared by preview and PNG export.

pub fn clip_rect(rect: [i32; 4], width: u32, height: u32) -> Option<[usize; 4]> {
    let [left, top, right, bottom] = rect;
    let left = i64::from(left).clamp(0, i64::from(width)) as usize;
    let top = i64::from(top).clamp(0, i64::from(height)) as usize;
    let right = i64::from(right).clamp(0, i64::from(width)) as usize;
    let bottom = i64::from(bottom).clamp(0, i64::from(height)) as usize;
    (right > left && bottom > top).then_some([left, top, right, bottom])
}

fn byte_len(width: u32, height: u32) -> Result<usize, &'static str> {
    (width as usize).checked_mul(height as usize)
        .and_then(|size| size.checked_mul(4)).ok_or("frame size overflow")
}

pub fn paint_snapshot(
    target: &mut [u8], original: &[u8], dimmed: &[u8],
    width: u32, height: u32, selection: Option<[i32; 4]>,
) -> Result<(), &'static str> {
    let size = byte_len(width, height)?;
    if target.len() != size || original.len() != size || dimmed.len() != size {
        return Err("frame buffer size mismatch");
    }
    target.copy_from_slice(dimmed);
    if let Some([left, top, right, bottom]) = selection.and_then(|r| clip_rect(r, width, height)) {
        let stride = width as usize * 4;
        for y in top..bottom {
            let start = y * stride + left * 4;
            let end = y * stride + right * 4;
            target[start..end].copy_from_slice(&original[start..end]);
        }
    }
    Ok(())
}

pub fn crop_rgba(
    bgra: &[u8], width: u32, height: u32, selection: [i32; 4],
) -> Result<Vec<u8>, &'static str> {
    if bgra.len() != byte_len(width, height)? {
        return Err("frame buffer size mismatch");
    }
    let [left, top, right, bottom] = clip_rect(selection, width, height).ok_or("empty selection")?;
    let mut rgba = Vec::with_capacity((right - left) * (bottom - top) * 4);
    for y in top..bottom {
        let start = (y * width as usize + left) * 4;
        let end = (y * width as usize + right) * 4;
        for pixel in bgra[start..end].chunks_exact(4) {
            // GDI's BI_RGB alpha byte is unspecified, screenshots are opaque.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Vec<u8> {
        (0..8).flat_map(|y| (0..8).flat_map(move |x| [x, y, 100, 255])).collect()
    }

    #[test]
    fn selection_preserves_original_rows_and_only_dims_outside() {
        let original = source();
        let dimmed = vec![9; original.len()];
        let mut preview = vec![0; original.len()];
        paint_snapshot(&mut preview, &original, &dimmed, 8, 8, Some([2, 1, 5, 3])).unwrap();
        for y in 0..8 {
            for x in 0..8 {
                let offset = (y * 8 + x) * 4;
                let expected = if (2..5).contains(&x) && (1..3).contains(&y) {
                    &original[offset..offset + 4]
                } else { &dimmed[offset..offset + 4] };
                assert_eq!(&preview[offset..offset + 4], expected);
            }
        }
    }

    #[test]
    fn png_crop_matches_the_bright_preview_without_red_blue_swap() {
        assert_eq!(crop_rgba(&source(), 8, 8, [2, 1, 4, 3]).unwrap(),
            vec![100, 1, 2, 255, 100, 1, 3, 255, 100, 2, 2, 255, 100, 2, 3, 255]);
    }

    #[test]
    fn dragging_outside_frame_clips_instead_of_reading_invalid_memory() {
        assert_eq!(clip_rect([-20, -10, 2, 1], 8, 8), Some([0, 0, 2, 1]));
        assert_eq!(crop_rgba(&source(), 8, 8, [-20, -10, 2, 1]).unwrap(),
            vec![100, 0, 0, 255, 100, 0, 1, 255]);
        assert_eq!(clip_rect([8, 8, 12, 12], 8, 8), None);
        assert_eq!(clip_rect([4, 4, 4, 7], 8, 8), None);
    }

    #[test]
    fn invalid_buffers_and_empty_rectangles_are_rejected() {
        assert!(crop_rgba(&[], 8, 8, [0, 0, 1, 1]).is_err());
        assert!(crop_rgba(&source(), 8, 8, [5, 5, 3, 3]).is_err());
        assert!(paint_snapshot(&mut [], &source(), &source(), 8, 8, None).is_err());
    }
}
