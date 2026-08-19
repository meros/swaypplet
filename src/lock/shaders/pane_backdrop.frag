#version 330 core

// Pass 1: the wallpaper as it appears *behind this pane*, scrim included.
//
// The pane sits over the full-screen `Picture`, so this has to reproduce that
// picture's ContentFit::Cover placement exactly or the refraction would bend
// a differently-scaled copy of the wallpaper and the seam at the card's edge
// would show. `uCover` is the same rect `cover_rect()` hands the GSK path, in
// device pixels and pane-local coordinates, so the two backends sample
// identical pixels.
//
// Deliberately *without* the scrim. The scrim is what the picture behind the
// card is dimmed by, and the glass needs both: the dimmed version to match
// against at the corners, and the undimmed one to refract, because dimming
// before refracting crushes exactly the contrast the lens exists to bend.
// The glass pass applies it to the one and not the other.

out vec4 fragColor;

uniform sampler2D uWallpaper;
uniform vec2  uResolution;   // buffer size, device px (pane plus 2*margin)
uniform float uMargin;       // device px of wallpaper captured outside the pane
uniform vec4  uCover;        // xy = texture origin, zw = texture size, pane-local
uniform float uHasWallpaper;

void main() {
    // Pane-local, y down, the coordinate system every widget bound in this
    // file is already expressed in. The buffer is bigger than the pane by
    // `uMargin` on every side, so pane-local coordinates here run negative
    // in the margin — which is the point: the glass needs wallpaper from
    // *outside* its own edge to reflect and to refract.
    vec2 p = vec2(gl_FragCoord.x, uResolution.y - gl_FragCoord.y) - uMargin;

    vec3 col = vec3(0.0);
    if (uHasWallpaper > 0.5 && uCover.z > 0.5 && uCover.w > 0.5) {
        // Row 0 of the upload is the top of the image and p.y grows downward,
        // so this needs no flip.
        vec2 uv = (p - uCover.xy) / uCover.zw;
        col = texture(uWallpaper, clamp(uv, 0.0, 1.0)).rgb;
    }
    fragColor = vec4(col, 1.0);
}
