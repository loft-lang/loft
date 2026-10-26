/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * <string.h> for a `[c] shim` compiled to wasm (loft_wasm_libc.c has the story). */
#ifndef LOFT_WASM_STRING_H
#define LOFT_WASM_STRING_H
#include <stddef.h>

void *memcpy(void *dst, const void *src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);

size_t loft_w_strlen(const char *s);
int loft_w_strcmp(const char *a, const char *b);
int loft_w_strncmp(const char *a, const char *b, size_t n);
char *loft_w_strcpy(char *dst, const char *src);
char *loft_w_strncpy(char *dst, const char *src, size_t n);
char *loft_w_strcat(char *dst, const char *src);
char *loft_w_strchr(const char *s, int c);
char *loft_w_strrchr(const char *s, int c);
char *loft_w_strstr(const char *h, const char *n);
void *loft_w_memchr(const void *s, int c, size_t n);

#define strlen loft_w_strlen
#define strcmp loft_w_strcmp
#define strncmp loft_w_strncmp
#define strcpy loft_w_strcpy
#define strncpy loft_w_strncpy
#define strcat loft_w_strcat
#define strchr loft_w_strchr
#define strrchr loft_w_strrchr
#define strstr loft_w_strstr
#define memchr loft_w_memchr

#endif
