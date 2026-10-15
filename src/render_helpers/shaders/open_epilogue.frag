
void main() {
    vec3 coords_geo = neori_input_to_geo * vec3(neori_v_coords, 1.0);
    vec3 size_geo = vec3(neori_geo_size, 1.0);

    vec4 color = open_color(coords_geo, size_geo);

    color = color * neori_alpha;

#if defined(DEBUG_FLAGS)
    if (neori_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
