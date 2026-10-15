precision highp float;

#if defined(DEBUG_FLAGS)
uniform float neori_tint;
#endif

varying vec2 neori_v_coords;
uniform vec2 neori_size;

uniform mat3 neori_input_to_geo;
uniform vec2 neori_geo_size;

uniform sampler2D neori_tex;
uniform mat3 neori_geo_to_tex;

uniform float neori_progress;
uniform float neori_clamped_progress;
uniform float neori_random_seed;

uniform float neori_alpha;
uniform float neori_scale;

