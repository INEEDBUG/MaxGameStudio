/// Linear FFmpeg expression for two or more time-ordered points.
/// Keep nesting shallow for projects with hundreds of keyframes.
pub(super) fn interpolate(points: &[(f64, f64)], time: &str) -> String {
    if let [(start, before), (end, after)] = points {
        let delta = (end - start).max(0.0001);
        return format!("{before:.6}+({after:.6}-{before:.6})*({time}-{start:.6})/{delta:.6}");
    }
    let middle = points.len() / 2;
    format!(
        "if(lt({time},{:.6}),{},{})",
        points[middle].0,
        interpolate(&points[..=middle], time),
        interpolate(&points[middle..], time)
    )
}
