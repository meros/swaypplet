#version 330 core

// Pass 2: liquid glass.
//
// The whole effect is one screen-space fragment shader over a merged signed
// distance field. Nothing is baked: the height field, its normal, the
// refracted sample points and the light concentration are all derived per
// pixel from the same SDF, so the shapes can move, merge and ripple and the
// optics follow without any precomputation.
//
// The chain per pixel is
//
//   merged SDF  ->  height field  ->  normal  ->  Snell refraction
//                                  ->  Laplacian -> light concentration
//
// and the refraction is spectral: instead of the usual three-tap RGB
// chromatic-aberration hack, N wavelengths across 400-700 nm each get their
// own index of refraction from Cauchy's equation and their own sample of the
// backdrop, recombined through a fit to the CIE 1931 colour matching
// functions. That is what makes the bezel throw a real prism edge rather
// than a red/blue fringe.

#define MAX_SHAPES  8
#define MAX_RIPPLES 4
#define MAX_SPECTRAL 24

in vec2 vUv;
out vec4 fragColor;

uniform sampler2D uBackdrop;
uniform vec2  uResolution;
uniform float uTime;
uniform float uMaxLod;

// Shapes live in UI space: origin top-left, y down, same as GTK pointer
// events, so the Rust side never has to flip anything.
uniform int   uShapeCount;
uniform vec2  uShapePos[MAX_SHAPES];
uniform vec2  uShapeHalf[MAX_SHAPES];
uniform float uShapeRadius[MAX_SHAPES];
uniform float uShapeRot[MAX_SHAPES];

uniform float uMerge;      // smooth-union radius, px
uniform float uBezel;      // width of the sloped rim, px
uniform float uThickness;  // height of the slab, px
uniform int   uProfile;    // 0 convex circle, 1 squircle, 2 concave, 3 lip
uniform float uIor;
uniform float uDispersion;
uniform int   uSamples;    // spectral taps
uniform float uFrost;
uniform float uSpecular;
uniform float uShine;
uniform vec3  uLightDir;
uniform float uFresnel;
uniform float uLensGain;
uniform vec4  uTint;
uniform float uShadow;
uniform float uRefract;    // artistic scale on the physical offset
uniform float uEdgeLight;
uniform float uNoise;
uniform float uDebug;      // 0 off, 1 normals, 2 height, 3 SDF, 4 lens gain

uniform int   uRippleCount;
uniform vec3  uRipple[MAX_RIPPLES]; // xy = origin in UI px, z = age in seconds
uniform float uRippleAmp;

const vec3 VIEW = vec3(0.0, 0.0, 1.0);
const vec3 INCIDENT = vec3(0.0, 0.0, -1.0);

// ---------------------------------------------------------------- sampling

vec2 toUv(vec2 uiPx) {
    return vec2(uiPx.x, uResolution.y - uiPx.y) / uResolution;
}

vec3 backdropAt(vec2 uiPx, float lod) {
    return textureLod(uBackdrop, toUv(uiPx), lod).rgb;
}

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

// ------------------------------------------------------------- distance field

float sdRoundRect(vec2 p, vec2 b, float r) {
    r = min(r, min(b.x, b.y));
    vec2 q = abs(p) - b + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// Polynomial smooth minimum. This is the "liquid" in liquid glass: two
// shapes that come within uMerge of each other fuse through a continuous
// neck rather than intersecting, and because every optical term below is
// derived from this one field, the merged blob refracts as a single piece of
// glass instead of two overlapping ones.
float smin(float a, float b, float k) {
    if (k <= 0.001) return min(a, b);
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

float sceneSDF(vec2 p) {
    float d = 1.0e6;
    for (int i = 0; i < MAX_SHAPES; i++) {
        if (i >= uShapeCount) break;
        vec2 q = p - uShapePos[i];
        float c = cos(uShapeRot[i]);
        float s = sin(uShapeRot[i]);
        q = vec2(c * q.x + s * q.y, -s * q.x + c * q.y);
        d = smin(d, sdRoundRect(q, uShapeHalf[i], uShapeRadius[i]), uMerge);
    }
    return d;
}

// ---------------------------------------------------------------- height field

// The four surface profiles are the ones both kube.io and scenefx-enhanced
// converged on, kept under the same names so the demo is comparable to the
// prior art. t = 0 at the outer edge, 1 at the top of the bezel.
float profile(float t) {
    t = clamp(t, 0.0, 1.0);
    if (uProfile == 0) {
        float u = 1.0 - t;
        return sqrt(max(1.0 - u * u, 0.0));          // convex circle
    } else if (uProfile == 1) {
        float u = 1.0 - t;
        return pow(max(1.0 - u * u * u * u, 0.0), 0.25); // convex squircle
    } else if (uProfile == 2) {
        return 1.0 - sqrt(max(1.0 - t * t, 0.0));    // concave
    }
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); // lip (smootherstep)
}

float heightAt(vec2 p) {
    float d = sceneSDF(p);
    float t = clamp(-d / max(uBezel, 0.5), 0.0, 1.0);
    float h = uThickness * profile(t);

    if (uRippleCount > 0) {
        // Ripples deform the top surface, not the colour. Everything
        // downstream, refraction included, is recomputed from the deformed
        // height, so a click sends a real travelling lens across the glass.
        float inside = 1.0 - smoothstep(-10.0, 0.0, d);
        for (int i = 0; i < MAX_RIPPLES; i++) {
            if (i >= uRippleCount) break;
            float age = uRipple[i].z;
            if (age < 0.0) continue;
            float r = length(p - uRipple[i].xy);
            float x = r - age * 760.0;                    // wavefront, px/s
            float env = exp(-x * x / (2.0 * 78.0 * 78.0)) // packet width
                      * exp(-age * 2.4)                   // decay in time
                      * exp(-r / 620.0);                  // decay in space
            h += uRippleAmp * sin(x * 0.055) * env * inside;
        }
    }
    return h;
}

// --------------------------------------------------------------- spectral

// Wyman, Sloan & Shirley (JCGT 2013) multi-lobe Gaussian fit to the CIE 1931
// colour matching functions. Cheap enough to run per wavelength per pixel.
float gaussSeg(float x, float mu, float s1, float s2) {
    float t = (x - mu) * ((x < mu) ? (1.0 / s1) : (1.0 / s2));
    return exp(-0.5 * t * t);
}

vec3 cieXyz(float nm) {
    float x = gaussSeg(nm, 442.0, 22.4, 34.0) * 0.362
            + gaussSeg(nm, 599.8, 37.9, 31.0) * 1.056
            + gaussSeg(nm, 501.1, 20.4, 26.2) * -0.065;
    float y = gaussSeg(nm, 568.8, 46.9, 40.5) * 0.821
            + gaussSeg(nm, 530.9, 16.3, 31.1) * 0.286;
    float z = gaussSeg(nm, 437.0, 11.8, 36.0) * 1.217
            + gaussSeg(nm, 459.0, 26.0, 13.8) * 0.681;
    return vec3(x, y, z);
}

vec3 xyzToLinearSrgb(vec3 c) {
    return vec3(
        dot(c, vec3( 3.2406, -1.5372, -0.4986)),
        dot(c, vec3(-0.9689,  1.8758,  0.0415)),
        dot(c, vec3( 0.0557, -0.2040,  1.0570))
    );
}

vec3 spectralWeight(float nm) {
    return max(xyzToLinearSrgb(cieXyz(nm)), vec3(0.0));
}

// ------------------------------------------------------------------- main

void main() {
    vec2 p = vec2(gl_FragCoord.x, uResolution.y - gl_FragCoord.y);
    float d = sceneSDF(p);

    // Contact shadow, sampled before the early-out so it survives outside
    // the glass. Offset downward in UI space.
    float sdShadow = sceneSDF(p - vec2(0.0, 18.0));
    float shadow = uShadow * (1.0 - smoothstep(-4.0, 34.0, sdShadow));

    vec3 bg = backdropAt(p, 0.0) * (1.0 - shadow);

    // Almost every pixel is outside the glass; skipping the seven SDF
    // evaluations and the spectral loop for those is what keeps this
    // fullscreen at native resolution.
    if (d > 1.5) {
        fragColor = vec4(bg, 1.0);
        return;
    }

    // Height field, its gradient and its Laplacian, all from the same five
    // taps. Central differences rather than an analytic derivative because
    // the field also carries the ripples and the smooth-union necks.
    const float e = 1.25;
    float h0  = heightAt(p);
    float hx1 = heightAt(p + vec2(e, 0.0));
    float hx0 = heightAt(p - vec2(e, 0.0));
    float hy1 = heightAt(p + vec2(0.0, e));
    float hy0 = heightAt(p - vec2(0.0, e));

    vec2 grad = vec2(hx1 - hx0, hy1 - hy0) / (2.0 * e);
    float lap = (hx1 + hx0 + hy1 + hy0 - 4.0 * h0) / (e * e);
    vec3 n = normalize(vec3(-grad, 1.0));

    // Frost grows with the path length through the glass: the thick middle
    // is milkier than the thin rim, which is how real frosted glass reads.
    float depth = clamp(h0 / max(uThickness, 1.0), 0.0, 1.0);
    float lod = clamp(uFrost * (0.35 + 0.65 * depth), 0.0, uMaxLod);
    lod += (hash21(p + uTime) - 0.5) * 0.25 * step(0.01, lod);

    // Spectral refraction. One index of refraction, one ray and one backdrop
    // sample per wavelength; the CIE weights decide how much each lands in
    // R, G and B.
    int k = clamp(uSamples, 1, MAX_SPECTRAL);
    vec3 acc = vec3(0.0);
    vec3 wsum = vec3(0.0);
    for (int i = 0; i < MAX_SPECTRAL; i++) {
        if (i >= k) break;
        float f = (k == 1) ? 0.5 : float(i) / float(k - 1);
        float nm = mix(400.0, 700.0, f);

        // Cauchy: n(l) = A + B/l^2, anchored so uIor is the index at the
        // sodium D line, which is the number glass is normally quoted at.
        float lu = nm * 0.001;              // micrometres
        const float lref = 0.5893;
        float ior = uIor + uDispersion * (1.0 / (lu * lu) - 1.0 / (lref * lref));
        ior = max(ior, 1.0001);

        vec3 r = refract(INCIDENT, n, 1.0 / ior);
        // The ray enters the top surface at height h0 and has to reach the
        // backdrop plane, so it travels h0 downward: that descent is the
        // entire source of the lateral displacement. A flat top gives
        // r = (0,0,-1) and no offset at all, which is why only the bezel
        // lenses.
        vec2 off = (r.z < -1.0e-3) ? r.xy * (h0 / -r.z) : vec2(0.0);
        vec3 c = backdropAt(p + off * uRefract, lod);

        vec3 w = spectralWeight(nm);
        acc += w * c;
        wsum += w;
    }
    vec3 refr = acc / max(wsum, vec3(1.0e-4));

    // Schlick against the same backdrop used as a stand-in environment. A
    // client has nothing else to reflect, and at these grazing angles a
    // blurred, offset copy of what is behind reads correctly.
    float f0 = pow((1.0 - uIor) / (1.0 + uIor), 2.0);
    float fres = f0 + (1.0 - f0) * pow(clamp(1.0 - n.z, 0.0, 1.0), 5.0);
    vec3 rv = reflect(INCIDENT, n);
    vec3 env = backdropAt(p - rv.xy * uThickness * 2.2,
                          min(uMaxLod, lod + 3.0));

    // Light concentration. The Laplacian of the height field is the thin-lens
    // convergence term, and it falls out of the five taps already spent on
    // the normal: convex-down shoulders focus and brighten, convex-up rims
    // diverge and darken. This is the "lensing concentrates light" claim,
    // for free.
    float gain = clamp(1.0 - uLensGain * lap * 6.0, 0.35, 2.6);

    vec3 lightDir = normalize(uLightDir);
    vec3 hvec = normalize(lightDir + VIEW);
    float spec = pow(max(dot(n, hvec), 0.0), max(uShine, 1.0)) * uSpecular;

    // A thin bright line just inside the boundary, brightest where the rim
    // faces the light. Cheap, and it is most of what sells the material.
    float rimBand = exp(-pow(d + 1.6, 2.0) / (2.0 * 1.4 * 1.4));
    float facing = 0.35 + 0.65 * max(dot(normalize(vec2(-grad) + 1.0e-5),
                                         normalize(lightDir.xy + 1.0e-5)), 0.0);
    float rim = rimBand * facing * uEdgeLight;

    // Order matters: the lens gain and the tint belong to light that went
    // *through* the glass. The reflected component bounced off the top
    // surface and was never focused or tinted by the slab, so it is mixed in
    // after both, with Fresnel splitting the energy F / 1-F.
    vec3 glass = refr * gain;
    glass = mix(glass, uTint.rgb, uTint.a);
    glass = mix(glass, env, clamp(fres * uFresnel, 0.0, 1.0));
    glass += spec + rim;
    glass += (hash21(p * 1.7 + uTime * 60.0) - 0.5) * uNoise;

    if (uDebug > 0.5) {
        if (uDebug < 1.5)      glass = n * 0.5 + 0.5;
        else if (uDebug < 2.5) glass = vec3(h0 / max(uThickness, 1.0));
        else if (uDebug < 3.5) glass = (d < 0.0 ? vec3(0.1, 0.6, 1.0)
                                               : vec3(1.0, 0.4, 0.1))
                                       * (0.35 + 0.65 * fract(abs(d) / 24.0));
        else                   glass = vec3(clamp(gain - 1.0, 0.0, 1.0),
                                            0.0,
                                            clamp(1.0 - gain, 0.0, 1.0));
    }

    float cov = 1.0 - smoothstep(-1.0, 1.0, d);
    fragColor = vec4(mix(bg, glass, cov), 1.0);
}
