#version 330 core

// Attribute-less fullscreen triangle: three vertices, one dummy VAO, nothing
// to keep in sync with a resize.
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
