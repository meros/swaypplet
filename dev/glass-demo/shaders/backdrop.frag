#version 330 core

// Pass 1: everything the glass will later refract.
//
// This exists so the demo cannot cheat. A glass effect drawn straight over a
// static image can fake refraction by warping that one image; here the glass
// pass only ever sees an offscreen colour buffer whose contents move every
// frame, exactly like a compositor sampling its own already-composited
// output. If the refraction tracks the motion, it is screen-space and real.

in vec2 vUv;
out vec4 fragColor;

uniform sampler2D uWallpaper;
uniform vec2  uResolution;
uniform vec2  uWallpaperSize;
uniform float uTime;
uniform float uHasWallpaper;
uniform float uTestCard;   // 0 = wallpaper only, 1 = full test card

const float TAU = 6.28318530718;

// Cover-fit, matching ContentFit::Cover: scale to fill, centre the overflow.
vec3 wallpaper(vec2 px) {
    if (uHasWallpaper < 0.5 || uWallpaperSize.x < 1.0) {
        return vec3(0.05, 0.06, 0.09);
    }
    float scale = max(uResolution.x / uWallpaperSize.x,
                      uResolution.y / uWallpaperSize.y);
    vec2 draw = uWallpaperSize * scale;
    vec2 uv = (px - (uResolution - draw) * 0.5) / draw;
    // GdkTexture rows arrive top-down; the framebuffer is bottom-up.
    uv.y = 1.0 - uv.y;
    return texture(uWallpaper, clamp(uv, 0.0, 1.0)).rgb;
}

float lineMask(float v, float period, float width, float feather) {
    float f = abs(mod(v, period) - period * 0.5);
    return 1.0 - smoothstep(width, width + feather, f);
}

void main() {
    vec2 px = gl_FragCoord.xy;
    vec3 col = wallpaper(px);

    if (uTestCard > 0.001) {
        // A drifting grid: straight lines are the only honest witness to a
        // lens. If they bend under the glass and stay straight beside it,
        // the displacement is real.
        vec2 g = px + vec2(uTime * 26.0, -uTime * 14.0);
        float grid = max(lineMask(g.x, 64.0, 0.8, 1.2),
                         lineMask(g.y, 64.0, 0.8, 1.2));
        float fine = max(lineMask(g.x, 16.0, 0.4, 0.9),
                         lineMask(g.y, 16.0, 0.4, 0.9)) * 0.35;

        // Saturated bars give the spectral pass something to split.
        float band = px.x / uResolution.x * 6.0 + uTime * 0.35;
        vec3 hue = 0.5 + 0.5 * cos(TAU * (fract(band) + vec3(0.0, 0.33, 0.67)));

        vec2 c = vec2(0.5 + 0.34 * sin(uTime * 0.53),
                      0.5 + 0.28 * cos(uTime * 0.41)) * uResolution;
        float blob = exp(-length(px - c) / 190.0);

        vec3 card = mix(vec3(0.02, 0.03, 0.05), hue, 0.55);
        card = mix(card, vec3(1.0), clamp(grid + fine, 0.0, 1.0) * 0.9);
        card += vec3(1.0, 0.85, 0.6) * blob * 0.8;

        col = mix(col, card, uTestCard);
    }

    fragColor = vec4(col, 1.0);
}
