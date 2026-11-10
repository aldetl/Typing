#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <string.h>
#include <stdlib.h>
#include "../../../inputsim/include/inputsim.h"

static INIT_ONCE initialization = INIT_ONCE_STATIC_INIT;
static HMODULE real_module;
static LONG rejected;
static uint32_t (__cdecl *real_key)(uint32_t, inputsim_event_v1 *);
static uint32_t (__cdecl *real_char)(uint32_t, inputsim_character_v1 *);
static uint32_t (__cdecl *real_submit)(const inputsim_event_v1 *, uint32_t, uintptr_t);

static BOOL CALLBACK initialize(PINIT_ONCE once, PVOID parameter, PVOID *context) {
    wchar_t *path;
    FARPROC address;
    DWORD capacity = GetEnvironmentVariableW(L"TYPING_TEST_INPUTSIM", NULL, 0);
    DWORD length;
    (void)once; (void)parameter; (void)context;
    if (!capacity || capacity > 32768) return FALSE;
    path = (wchar_t *)malloc(capacity * sizeof(wchar_t));
    if (!path) return FALSE;
    length = GetEnvironmentVariableW(L"TYPING_TEST_INPUTSIM", path, capacity);
    if (!length || length >= capacity) { free(path); return FALSE; }
    real_module = LoadLibraryExW(path, NULL,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    free(path);
    if (!real_module) return FALSE;
#define RESOLVE(name, target) \
    address = GetProcAddress(real_module, name); \
    if (!address) { FreeLibrary(real_module); real_module = NULL; return FALSE; } \
    memcpy(&target, &address, sizeof(target))
    RESOLVE("resolve_key_v1", real_key);
    RESOLVE("resolve_char_v1", real_char);
    RESOLVE("submit_v1", real_submit);
#undef RESOLVE
    return TRUE;
}

static int ready(void) {
    return InitOnceExecuteOnce(&initialization, initialize, NULL, NULL) != 0;
}

uint32_t __cdecl resolve_key_v1(uint32_t key, inputsim_event_v1 *output) {
    return ready() ? real_key(key, output) : INPUTSIM_UNAVAILABLE;
}
uint32_t __cdecl resolve_char_v1(uint32_t character, inputsim_character_v1 *output) {
    return ready() ? real_char(character, output) : INPUTSIM_UNAVAILABLE;
}
uint32_t __cdecl submit_v1(const inputsim_event_v1 *event,
        uint32_t direction, uintptr_t tag) {
    if (!ready()) return INPUTSIM_NOT_INSERTED;
    if (event && event->kind == INPUTSIM_SCAN_CODE && event->code == 0x36 &&
        direction == INPUTSIM_UP && InterlockedCompareExchange(&rejected, 1, 0) == 0) {
        return INPUTSIM_NOT_INSERTED;
    }
    return real_submit(event, direction, tag);
}
