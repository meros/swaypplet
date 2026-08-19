#version 330 core

// Pass 2: the card, as a slab of glass over the wallpaper behind it.
//
// This is the lock screen's half of the technique demonstrated in
// dev/glass-demo, cut down to what one pane needs: a single rounded rect
// rather than a merged field, and no ripples.
//
// The output is opaque, and the rounded corners are anti-aliased by blending
// against this pane's own copy of the backdrop rather than by writing alpha.
// That is not a shortcut: pass 1 already reproduces the wallpaper and scrim
// exactly as the picture behind the pane draws them, so the corner nubs come
// out pixel-identical to what they cover, and the edge quality stops
// depending on whether GTK reads this framebuffer as premultiplied.
//
// Everything optical derives from one height field over the pane's own SDF:
//
//   SDF ──▶ height ──▶ normal ──▶ Snell refraction ──▶ spectral samples
//                  └─▶ Laplacian ─▶ light concentration
//
// `uT` is the materialize ramp, the GL backend's answer to the GSK path's
// blur sigma: at 0 the pane is a crisp scrimmed copy of the wallpaper, which
// is pixel-identical to the picture around it, and at 1 it is full glass. The
// card fades in over the same 300 ms.

#define MAX_SPECTRAL 16

out vec4 fragColor;

uniform sampler2D uBackdrop;
uniform vec2  uResolution;   // pane size, device px
uniform vec2  uBuffer;       // backdrop buffer size, pane plus 2*margin
uniform float uMargin;
uniform float uMaxLod;
uniform float uT;

uniform vec2  uHalf;        // pane half-extents, device px
uniform float uRadius;      // corner radius, device px
uniform float uBezel;
uniform float uThickness;
uniform float uIor;
uniform int   uProfile;
uniform float uDispersion;
uniform int   uSamples;
uniform float uFrost;
uniform float uSpecular;
uniform float uShine;
uniform vec3  uLightDir;
uniform float uFresnel;
uniform float uReflectLod;
uniform float uLensGain;
uniform float uEdgeLight;
uniform float uNoise;

// ── Directions being explored ────────────────────────────────────────────
// Each defaults to 0 and costs nothing when off, so the zoo is one binary.
//
// Apple's own account of Liquid Glass names two behaviours this started
// without, and their 2026 revision doubled down on the second: the tint and
// dynamic range shift to keep content legible over anything, and complex
// content behind the glass gets diffused to create separation. Both are
// legibility mechanisms rather than decoration, which is why they are the
// two most worth having on a password field.
uniform float uAdaptive;      // shift tone away from the backdrop's luminance
uniform float uAdaptiveFrost; // scatter more where the backdrop is busy
uniform float uIridescence;   // thin-film colour at grazing angles
uniform float uFilm;          // film thickness, sets the fringe spacing
uniform float uCaustic;       // sharpened convergence band inside the rim
uniform float uInnerShadow;   // contact darkening just inside the edge
uniform float uSparkle;       // micro-glints on the bevel
uniform float uBump;          // procedural roughness on the surface
uniform vec4  uScrim;       // matches .lock-scrim, applied here not in pass 1
uniform vec3  uAbsorb;
uniform float uAbsorbFloor;
uniform float uHaze;
/// How much of the scrim the glass itself takes. Below 1 the card reads as a
/// window onto the undimmed wallpaper, which is what a lens should look like.
uniform float uScrimInGlass;

const vec3 VIEW = vec3(0.0, 0.0, 1.0);
const vec3 INCIDENT = vec3(0.0, 0.0, -1.0);

// Pane-local (y down) to a sample of the backdrop buffer. The buffer is
// inset by `uMargin`, so this reaches a margin's worth outside the pane
// before it clamps.
vec3 backdropAt(vec2 p, float lod) {
    vec2 b = p + uMargin;
    vec2 uv = vec2(b.x, uBuffer.y - b.y) / uBuffer;
    return textureLod(uBackdrop, uv, lod).rgb;
}

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float sdRoundRect(vec2 p, vec2 b, float r) {
    r = min(r, min(b.x, b.y));
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float paneSDF(vec2 p) {
    return sdRoundRect(p - uResolution * 0.5, uHalf, uRadius);
}

// Convex squircle, as a function of distance alone. Of the four profiles the
// demo carries this is the one that suits an 18px-radius card: the circle
// profile creases visibly where the bezel meets the flat top at this size.
//
// The bezel is clamped to the corner radius, and that is a correctness
// constraint rather than taste. A rounded rect's distance field has a cone
// singularity at the centre of each corner arc, exactly `uRadius` inside the
// corner, where the gradient direction is undefined and the second derivative
// diverges. If the sloped band reaches that point, the Laplacian below blows
// up and the light-concentration term clamps to its floor, leaving a dark dot
// in each corner. Keeping the band no wider than the radius puts the
// singularity in the flat top, where the profile's first and second
// derivatives are both zero and it cannot matter.
float profileHeight(float d) {
    float t = clamp(-d / max(min(uBezel, uRadius), 0.5), 0.0, 1.0);
    float u = 1.0 - t;
    float z;
    if (uProfile == 0) {
        z = sqrt(max(1.0 - u * u, 0.0));                  // convex circle
    } else if (uProfile == 1) {
        z = pow(max(1.0 - u * u * u * u, 0.0), 0.25);     // convex squircle
    } else if (uProfile == 2) {
        z = 1.0 - sqrt(max(1.0 - t * t, 0.0));            // concave
    } else {
        z = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);    // lip
    }
    return uThickness * z;
}

float gaussSeg(float x, float mu, float s1, float s2) {
    float t = (x - mu) * ((x < mu) ? (1.0 / s1) : (1.0 / s2));
    return exp(-0.5 * t * t);
}

// Wyman, Sloan & Shirley (JCGT 2013) fit to the CIE 1931 colour matching
// functions, so each wavelength lands in R, G and B by how the eye sees it.
vec3 spectralWeight(float nm) {
    vec3 c = vec3(
        gaussSeg(nm, 442.0, 22.4, 34.0) * 0.362
            + gaussSeg(nm, 599.8, 37.9, 31.0) * 1.056
            + gaussSeg(nm, 501.1, 20.4, 26.2) * -0.065,
        gaussSeg(nm, 568.8, 46.9, 40.5) * 0.821
            + gaussSeg(nm, 530.9, 16.3, 31.1) * 0.286,
        gaussSeg(nm, 437.0, 11.8, 36.0) * 1.217
            + gaussSeg(nm, 459.0, 26.0, 13.8) * 0.681
    );
    return max(vec3(dot(c, vec3(3.2406, -1.5372, -0.4986)),
                    dot(c, vec3(-0.9689, 1.8758, 0.0415)),
                    dot(c, vec3(0.0557, -0.2040, 1.0570))),
               vec3(0.0));
}

void main() {
    vec2 p = vec2(gl_FragCoord.x, uResolution.y - gl_FragCoord.y);
    float d = paneSDF(p);

    // Coverage over exactly one pixel of the distance field, whatever the
    // scale factor: fwidth(d) is how far d moves between neighbouring
    // fragments, so the ramp is one pixel wide on a 1x panel and one physical
    // pixel wide on a HiDPI one. A fixed +/-1 would soften as scale rose.
    float aa = max(fwidth(d), 1.0e-4);
    float cov = 1.0 - smoothstep(-aa, aa, d);

    // What the pane covers, undistorted and then dimmed exactly as the
    // picture behind the card is. Outside the rounded corners this is what
    // gets drawn, and it has to match that picture pixel for pixel.
    vec3 flat_ = mix(backdropAt(p, 0.0), uScrim.rgb, uScrim.a);
    if (cov <= 0.0 || uT <= 0.001) {
        // uT 0 is the start of the materialize ramp: crisp, identical to the
        // picture around the card, exactly as the GSK path at sigma 0.
        fragColor = vec4(flat_, 1.0);
        return;
    }

    // Five taps of the distance field, and every differential term below is
    // derived from them: heights are a pure function of distance, so sampling
    // the SDF once per tap and mapping afterwards costs four fewer SDF
    // evaluations than differencing the height field directly.
    const float e = 1.25;
    float dx1 = paneSDF(p + vec2(e, 0.0));
    float dx0 = paneSDF(p - vec2(e, 0.0));
    float dy1 = paneSDF(p + vec2(0.0, e));
    float dy0 = paneSDF(p - vec2(0.0, e));

    float h0 = profileHeight(d);
    float hx1 = profileHeight(dx1);
    float hx0 = profileHeight(dx0);
    float hy1 = profileHeight(dy1);
    float hy0 = profileHeight(dy0);

    vec2 grad = vec2(hx1 - hx0, hy1 - hy0) / (2.0 * e);
    float lap = (hx1 + hx0 + hy1 + hy0 - 4.0 * h0) / (e * e);
    vec3 n = normalize(vec3(-grad, 1.0));

    // A true distance field has a unit gradient everywhere it is smooth, so a
    // gradient shorter than that means these five taps straddle a crease. The
    // bezel clamp above should keep that from happening at all; this makes the
    // second-derivative term degrade quietly rather than sharply if some
    // future geometry change breaks the invariant.
    float sane = smoothstep(0.35, 0.85,
                            length(vec2(dx1 - dx0, dy1 - dy0) / (2.0 * e)));

    // Bumpy glass: perturb the surface before anything reads it, so the
    // refraction, the reflection and the specular all agree about a rough
    // surface rather than a smooth one wearing noise.
    if (uBump > 0.0001) {
        float e2 = 2.0;
        float nb = hash21(floor(p / 3.0));
        float nbx = hash21(floor((p + vec2(e2, 0.0)) / 3.0));
        float nby = hash21(floor((p + vec2(0.0, e2)) / 3.0));
        n = normalize(n + vec3(-(nbx - nb), -(nby - nb), 0.0) * uBump * 4.0);
    }

    // Frost grows with path length through the glass: the thick middle is
    // milkier than the thin rim, as frosted glass reads.
    float depth = clamp(h0 / max(uThickness, 1.0), 0.0, 1.0);
    float lod = clamp(uFrost * (0.35 + 0.65 * depth), 0.0, uMaxLod);

    // Diffuse complex content. Where the backdrop is busy the difference
    // between a sharp and a scattered sample is large, which is a local
    // measure of detail for two taps — and detail is exactly what has to go
    // away for text on the glass to stay readable.
    vec3 coarse = backdropAt(p, min(uMaxLod, lod + 4.0));
    if (uAdaptiveFrost > 0.0001) {
        float busy = clamp(length(backdropAt(p, lod) - coarse) * 3.0, 0.0, 1.0);
        lod = clamp(lod + uAdaptiveFrost * busy * 3.0, 0.0, uMaxLod);
    }

    int k = clamp(uSamples, 1, MAX_SPECTRAL);
    vec3 acc = vec3(0.0);
    vec3 wsum = vec3(0.0);
    for (int i = 0; i < MAX_SPECTRAL; i++) {
        if (i >= k) break;
        float f = (k == 1) ? 0.5 : float(i) / float(k - 1);
        float nm = mix(400.0, 700.0, f);
        float lu = nm * 0.001;
        const float lref = 0.5893;
        float ior = max(uIor + uDispersion * (1.0 / (lu * lu) - 1.0 / (lref * lref)), 1.0001);

        vec3 r = refract(INCIDENT, n, 1.0 / ior);
        // The ray enters at height h0 and must descend that far to reach the
        // backdrop plane; that descent is the whole source of the offset. A
        // flat top gives no offset, so only the bezel lenses.
        vec2 off = (r.z < -1.0e-3) ? r.xy * (h0 / -r.z) : vec2(0.0);
        vec3 w = spectralWeight(nm);
        acc += w * backdropAt(p + off, lod);
        wsum += w;
    }
    // ---- transmitted: light that went through the slab ----------------
    vec3 trans = acc / max(wsum, vec3(1.0e-4));

    // The Laplacian of the height field is the thin-lens convergence term,
    // free from the five taps the normal already cost.
    trans *= clamp(1.0 - uLensGain * lap * 6.0 * sane, 0.45, 2.2);

    // Smoked, not clear. Beer-Lambert along the path through the slab:
    // transmission falls exponentially with how far the ray travelled inside
    // it, so the thick middle goes dark while the thin rim stays readable,
    // and the imbalance between the three coefficients is where the tint
    // comes from — no separate tint term, because absorption already is one.
    //
    // The floor matters as much as the coefficients. Path length goes to zero
    // at the very edge, and unsmoked glass there reads as a bright wire
    // around the card.
    float path = uAbsorbFloor + (1.0 - uAbsorbFloor) * depth;
    vec3 transmission = exp(-uAbsorb * path);
    trans *= transmission;

    // Forward scattering: the turbidity that makes it smoke rather than just
    // dark glass. Mixing toward a heavily blurred backdrop is the cheap
    // stand-in for light that bounced around inside before coming out, so it
    // is absorbed on the way out like everything else in here.
    trans = mix(trans, backdropAt(p, min(uMaxLod, lod + 4.0)) * transmission,
                uHaze * path);

    // The scrim dims the card body as it dims the screen, but only after the
    // optics: refraction and absorption both worked on full-contrast
    // wallpaper, which is the contrast the lens exists to bend.
    trans = mix(trans, uScrim.rgb, uScrim.a * uScrimInGlass);

    // ---- reflected: light that never got in ---------------------------
    //
    // Everything above belongs to the transmitted path and must not touch
    // this one. The reflected ray bounced off the top surface: it was never
    // absorbed by the slab, never scattered inside it, and never focused by
    // it. Mixing the reflection in before those terms — which is what this
    // did first — attenuates it by up to exp(-absorb) for no physical reason,
    // and then wants the Fresnel weight raised to compensate.
    //
    // The ray leaves the surface at height h0. Where the surface is steeper
    // than 45 degrees, rv.z stays negative (rv = (2·nz·nx, 2·nz·ny, 2·nz²−1),
    // so rv.z < 0 exactly when nz < 1/sqrt2) and it lands back on the
    // backdrop plane *outside* the shape, because rv.xy carries the sign of
    // the outward-tilting normal. That is the left edge showing what lies to
    // its left, the way the rim of a water droplet on a table does, and it is
    // the whole reason pass 1 captures a margin.
    //
    // Where the surface is shallower the ray leaves upward and there is
    // nothing above to sample. A client has no environment, so rather than
    // invent a highlight this falls back to the backdrop's average colour.
    float f0 = pow((1.0 - uIor) / (1.0 + uIor), 2.0);
    float fres = f0 + (1.0 - f0) * pow(clamp(1.0 - n.z, 0.0, 1.0), 5.0);

    vec3 rv = reflect(INCIDENT, n);
    vec3 ambient = backdropAt(p, uMaxLod);
    vec3 env = ambient;
    if (rv.z < -1.0e-3) {
        vec2 hit = p + rv.xy * (h0 / -rv.z);
        // Past the captured margin the sample clamps and stops meaning
        // anything, so it dissolves into the ambient instead. This also
        // catches rv.z approaching zero, where the ray goes near-horizontal
        // and the hit point runs away to infinity.
        float reach = length(hit - p);
        env = mix(backdropAt(hit, min(uMaxLod, lod + uReflectLod)),
                  ambient,
                  smoothstep(uMargin * 0.55, uMargin * 1.05, reach));
    }
    // What is reflected is the screen, and the screen is scrimmed. Sampling
    // the raw wallpaper here made the rim brighter than everything beside it,
    // which is most of why the edges read as lit rather than as glass.
    env = mix(env, uScrim.rgb, uScrim.a);

    // Fresnel splits the energy: F reflected, 1-F transmitted.
    vec3 glass = mix(trans, env, clamp(fres * uFresnel, 0.0, 1.0));

    // Thin-film interference: a coating a few hundred nanometres thick makes
    // the reflection's phase depend on angle, which is the soap-bubble
    // rainbow. Approximated by driving a hue cycle from the same grazing term
    // Fresnel uses, so it appears exactly where a real coating would show.
    if (uIridescence > 0.0001) {
        float phase = uFilm * (1.0 - n.z);
        vec3 film = 0.5 + 0.5 * cos(6.28318 * (phase + vec3(0.0, 0.33, 0.67)));
        glass += film * fres * uIridescence;
    }

    vec3 lightDir = normalize(uLightDir);
    glass += pow(max(dot(n, normalize(lightDir + VIEW)), 0.0), max(uShine, 1.0)) * uSpecular;

    // Glints: the bevel of real cut glass is never perfectly smooth, and the
    // flecks that catch the light are what says "cut" rather than "moulded".
    if (uSparkle > 0.0001) {
        float grain = hash21(floor(p * 0.7));
        float band = smoothstep(0.05, 0.35, depth) * (1.0 - smoothstep(0.6, 0.95, depth));
        glass += step(0.985, grain) * band * uSparkle;
    }

    // A thin bright line just inside the edge, brightest where the rim faces
    // the light. Cheap, and most of what sells the material.
    float facing = 0.35
        + 0.65 * max(dot(normalize(vec2(-grad) + 1.0e-5), normalize(lightDir.xy + 1.0e-5)), 0.0);
    glass += exp(-pow(d + 1.6, 2.0) / (2.0 * 1.4 * 1.4)) * facing * uEdgeLight;

    glass += (hash21(p) - 0.5) * uNoise;

    // Adaptive tone. Apple's stated mechanism for legibility, and the honest
    // reason to want it: over a bright backdrop the card has to darken and
    // over a dark one lighten, or a fixed tint is unreadable against half the
    // wallpapers in the world. Driven by the coarse sample, so it responds to
    // the region rather than to a pixel.
    if (uAdaptive > 0.0001) {
        float lum = dot(coarse, vec3(0.2126, 0.7152, 0.0722));
        glass *= 1.0 - uAdaptive * (lum - 0.35);
    }

    // Two cross-fades against the same crisp backdrop: uT so the materialize
    // ramp has no discontinuity at either end, then cov so the rounded corner
    // is anti-aliased.
    glass = mix(flat_, glass, clamp(uT, 0.0, 1.0));
    fragColor = vec4(mix(flat_, glass, cov), 1.0);
}
