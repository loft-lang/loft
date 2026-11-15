/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* The table a backend exports and the layer calls through. */
#ifndef LYR_BACKEND_H
#define LYR_BACKEND_H
#include <stdint.h>
typedef struct {
    int64_t version;              /* 1 */
    int64_t (*apply)(int64_t x);  /* the backend's own arithmetic */
    const char *name;
} lyr_backend;
#endif
