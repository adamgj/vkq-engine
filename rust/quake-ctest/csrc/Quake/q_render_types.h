/*
Copyright (C) 2026 vkqr-engine contributors

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.

See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program; if not, write to the Free Software
Foundation, Inc., 59 Temple Place - Suite 330, Boston, MA  02111-1307, USA.
*/

#ifndef __Q_RENDER_TYPES_H
#define __Q_RENDER_TYPES_H

// q_render_types.h -- the single entry point for Vulkan types in engine
// headers (Rust migration Phase 0, PLAN.md 4.1).
//
// Headers whose structs embed Vulkan handles (render.h, gl_model.h,
// gl_texmgr.h, gl_heap.h, glquake.h) include this instead of relying on
// quakedef.h to provide vulkan_core.h, so the "core" headers (q_types.h,
// protocol.h, bspfile.h, ...) stay bindgen-processable with no Vulkan on
// the include path.
//
// Phase 8 (renderer port) replaces this with proper hot/cold struct splits
// -- see entity_blas_t in render.h for the pattern -- after which the
// remaining engine structs carry no Vulkan handles at all.

#include <vulkan/vulkan_core.h>
#if VK_HEADER_VERSION < 162
#error Vulkan SDK too old
#endif

#endif /* __Q_RENDER_TYPES_H */
