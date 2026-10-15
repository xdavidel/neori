precision highp float;

#if defined(DEBUG_FLAGS)
uniform float neori_tint;
#endif

varying vec2 neori_v_coords;
uniform vec2 neori_size;

uniform mat3 neori_input_to_curr_geo;
uniform mat3 neori_curr_geo_to_prev_geo;
uniform mat3 neori_curr_geo_to_next_geo;
uniform vec2 neori_curr_geo_size;

uniform sampler2D neori_tex_prev;
uniform mat3 neori_geo_to_tex_prev;

uniform sampler2D neori_tex_next;
uniform mat3 neori_geo_to_tex_next;

uniform float neori_progress;
uniform float neori_clamped_progress;

uniform vec4 neori_corner_radius;
uniform float neori_clip_to_geometry;

uniform float neori_alpha;
uniform float neori_scale;

float neori_rounding_alpha(vec2 coords, vec2 size, vec4 corner_radius);
