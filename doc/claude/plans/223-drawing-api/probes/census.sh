#!/bin/bash
# How each user of the drawing libraries calls them: counts of call sites by kind.
[ $# -ge 1 ] || { echo "usage: census.sh <source dir>…  (a consumer's src, a library's src)"; exit 2; }
c() { grep -rhoE "$2" --include=*.loft "$1" 2>/dev/null | wc -l; }
printf '%-10s %6s %6s %6s %6s %6s %6s %6s %6s %6s\n' user draws sets state target painter shaders meshes textures input
for u in "$@"; do
  printf '%-10s %6s %6s %6s %6s %6s %6s %6s %6s %6s\n' "$(basename "$(dirname "$u")")" \
    "$(c $u '\.(draw|draw_mode|draw_indexed|draw_instanced|draw_all|draw_copies)\(')" \
    "$(c $u '\.set_(float|vec2|vec3|vec4|mat4|int)\(|\.(f1|f2|f4|mat4|f4_at)\(')" \
    "$(c $u '\.(enable|disable|blend|cull|depth_write|scissor|viewport|no_scissor)\(')" \
    "$(c $u '\.(render_to|render_to_window|begin|begin_window)\(')" \
    "$(c $u '\.(rect|image|sprite)\(')" \
    "$(c $u '\.(shader|pipeline)\(')" \
    "$(c $u '\.(upload|quad|buffer|instances|update)\(')" \
    "$(c $u '\.(texture|texture_from|load_texture|text_texture|depth_texture)\(')" \
    "$(c $u '\.(events|poll|key_down|mouse_x|typed|keys)\(')"
done
echo "-- uniform names set, most used:"
grep -rhoE '\.set_(float|vec2|vec3|vec4|mat4|int)\("[A-Za-z_0-9]+"' --include=*.loft "$@" \
  | sed -E 's/.*\("//; s/"//' | sort | uniq -c | sort -rn | head -14 | tr '\n' ' '
echo; echo "-- what each user imports:"
for u in "$@"; do
  echo "$u: $(grep -rhoE '^use (graphics|gpu[a-z_]*|stage|scene3d|text2d|drawing|input|imaging|mesh3d)\b' --include=*.loft $u 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
done
echo "-- state toggles by kind:"
grep -rhoE '\.(enable|disable|blend|cull|depth_write|scissor|viewport)\([A-Za-z_:.0-9]*' --include=*.loft "$@" | sed -E 's/^\.//' | sort | uniq -c | sort -rn | head -16 | tr '\n' ' '
echo
