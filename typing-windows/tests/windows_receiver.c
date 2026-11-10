#define WIN32_LEAN_AND_MEAN
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define CLIENT_ENABLE 1001
#define CLIENT_DISABLE 1002
#define CLIENT_QUIT 1003
typedef struct event_record { UINT key; int up; ULONG_PTR tag; } event_record;
static event_record delivered[16384], native_events[16384];
static unsigned delivered_count, native_count, character_count;
static wchar_t characters[512];
static HWND receiver, client, overlay;
static HANDLE client_process;
static DWORD client_pid;
static ULONG_PTR client_tag;
static UINT interrupt_control;
static UINT interrupt_message = WM_COMMAND;
static int overflow, driver_held[256];
static int send_key(UINT key, int up);

#define CHECK(condition) do { if (!(condition)) { \
    fprintf(stderr, "FAIL line %d: %s (Windows error %lu)\n", __LINE__, #condition, GetLastError()); \
    return 0; } } while (0)

static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM key, LPARAM data) {
    switch (message) {
    case WM_KEYDOWN: case WM_KEYUP: case WM_SYSKEYDOWN: case WM_SYSKEYUP:
        if (delivered_count == ARRAYSIZE(delivered)) { overflow = 1; return 0; }
        delivered[delivered_count].key = (UINT)key;
        delivered[delivered_count].up = message == WM_KEYUP || message == WM_SYSKEYUP;
        delivered[delivered_count++].tag = (ULONG_PTR)GetMessageExtraInfo();
        return 0;
    case WM_CHAR: case WM_SYSCHAR:
        if (character_count + 1 >= ARRAYSIZE(characters)) { overflow = 1; return 0; }
        characters[character_count++] = (wchar_t)key;
        characters[character_count] = 0;
        return 0;
    case WM_APP + 10:
        if (!send_key(VK_LCONTROL, 0)) overflow = 1;
        PostMessageW(client, interrupt_message, key, 0);
        return 0;
    default: return DefWindowProcW(window, message, key, data);
    }
}

static LRESULT CALLBACK observe(int code, WPARAM message, LPARAM data) {
    (void)message;
    if (code == HC_ACTION) {
        KBDLLHOOKSTRUCT *event = (KBDLLHOOKSTRUCT *)data;
        if (native_count < ARRAYSIZE(native_events)) {
            native_events[native_count].key = event->vkCode;
            native_events[native_count].up = !!(event->flags & LLKHF_UP);
            native_events[native_count++].tag = event->dwExtraInfo;
        } else { overflow = 1; }
        if (interrupt_control && event->vkCode == VK_RSHIFT &&
            !(event->flags & LLKHF_UP) && event->dwExtraInfo == client_tag) {
            PostMessageW(receiver, WM_APP + 10, interrupt_control, 0);
            interrupt_control = 0;
        }
    }
    return CallNextHookEx(NULL, code, message, data);
}

static void pump(DWORD milliseconds) {
    ULONGLONG end = GetTickCount64() + milliseconds;
    do {
        MSG message;
        while (PeekMessageW(&message, NULL, 0, 0, PM_REMOVE)) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if (GetTickCount64() >= end) break;
        MsgWaitForMultipleObjects(0, NULL, FALSE, 5, QS_ALLINPUT);
    } while (1);
}

/* Establish the test target only at setup/teardown boundaries. During a case,
   send_key must still fail immediately if feedback or another window takes focus. */
static int focus_receiver(const wchar_t *stage) {
    ULONGLONG deadline = GetTickCount64() + 60000;
    SetForegroundWindow(receiver);
    if (GetForegroundWindow() != receiver) {
        fwprintf(stdout, L"Activate the Typing integration receiver window (%s).\n", stage);
        fflush(stdout);
    }
    while (GetForegroundWindow() != receiver && GetTickCount64() < deadline) pump(10);
    if (GetForegroundWindow() != receiver) {
        fwprintf(stderr, L"FAIL receiver focus unavailable (%s); no input sent\n", stage);
        return 0;
    }
    SetFocus(receiver);
    return 1;
}

/* Desktop automation can still move the pointer while finishing activation.
   An optional caller-owned event hands the desktop to this test explicitly. */
static int wait_start_signal(void) {
    wchar_t name[256];
    DWORD length = GetEnvironmentVariableW(L"TYPING_TEST_START_EVENT", name, ARRAYSIZE(name));
    HANDLE signal;
    ULONGLONG deadline;
    DWORD result;
    if (!length) return 1;
    CHECK(length < ARRAYSIZE(name));
    signal = OpenEventW(SYNCHRONIZE, FALSE, name);
    CHECK(signal != NULL);
    fwprintf(stdout, L"INFO receiver ready; waiting for start event %s\n", name);
    fflush(stdout);
    deadline = GetTickCount64() + 60000;
    while ((result = WaitForSingleObject(signal, 0)) == WAIT_TIMEOUT && GetTickCount64() < deadline) pump(10);
    CloseHandle(signal);
    CHECK(result == WAIT_OBJECT_0);
    return 1;
}

static unsigned count(event_record *events, unsigned size, UINT key, int up, ULONG_PTR tag) {
    unsigned index, result = 0;
    for (index = 0; index < size; index++) {
        if (events[index].key == key && events[index].up == up && events[index].tag == tag) result++;
    }
    return result;
}

static int send_key(UINT key, int up) {
    INPUT event = {0};
    if (GetForegroundWindow() != receiver) {
        HWND foreground = GetForegroundWindow();
        DWORD foreground_pid = 0;
        wchar_t foreground_class[128] = {0};
        GetWindowThreadProcessId(foreground, &foreground_pid);
        GetClassNameW(foreground, foreground_class, ARRAYSIZE(foreground_class));
        fwprintf(stderr, L"Focus lost before key %u %s: foreground PID %lu class %s; client PID %lu visible %d\n",
            key, up ? L"up" : L"down", foreground_pid, foreground_class, client_pid, IsWindowVisible(client));
    }
    CHECK(GetForegroundWindow() == receiver);
    event.type = INPUT_KEYBOARD;
    event.ki.wVk = (WORD)key;
    event.ki.dwFlags = up ? KEYEVENTF_KEYUP : 0;
    CHECK(SendInput(1, &event, sizeof(event)) == 1);
    driver_held[key] = !up;
    return 1;
}

static int tap(UINT key) {
    CHECK(send_key(key, 0));
    CHECK(send_key(key, 1));
    pump(20);
    return 1;
}

static BOOL CALLBACK find_client(HWND window, LPARAM unused) {
    DWORD pid;
    wchar_t name[128];
    (void)unused;
    GetWindowThreadProcessId(window, &pid);
    GetClassNameW(window, name, ARRAYSIZE(name));
    if (pid == client_pid && wcscmp(name, L"TypingWindowsClient") == 0) client = window;
    if (pid == client_pid && wcscmp(name, L"TypingCursorFeedback") == 0) overlay = window;
    return TRUE;
}

static int title_contains(const wchar_t *text) {
    wchar_t title[512];
    if (!client) EnumWindows(find_client, 0);
    if (!client) return 0;
    GetWindowTextW(client, title, ARRAYSIZE(title));
    return wcsstr(title, text) != NULL;
}

static int wait_status(const wchar_t *text) {
    ULONGLONG deadline = GetTickCount64() + 5000;
    while (!title_contains(text) && GetTickCount64() < deadline &&
           WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT) pump(5);
    CHECK(title_contains(text));
    CHECK(!overflow);
    return 1;
}

static int wait_output(UINT key, unsigned downs, unsigned ups) {
    ULONGLONG deadline = GetTickCount64() + 5000;
    while ((count(delivered, delivered_count, key, 0, client_tag) < downs ||
            count(delivered, delivered_count, key, 1, client_tag) < ups) &&
           GetTickCount64() < deadline) pump(5);
    if(count(delivered,delivered_count,key,0,client_tag)!=downs || count(delivered,delivered_count,key,1,client_tag)!=ups) {
        fprintf(stderr,"Output key %u: expected %u/%u, delivered %u/%u, native %u/%u, tag %llu\n",key,downs,ups,
            count(delivered,delivered_count,key,0,client_tag),count(delivered,delivered_count,key,1,client_tag),
            count(native_events,native_count,key,0,client_tag),count(native_events,native_count,key,1,client_tag),(unsigned long long)client_tag);
    }
    CHECK(count(delivered, delivered_count, key, 0, client_tag) == downs);
    CHECK(count(delivered, delivered_count, key, 1, client_tag) == ups);
    CHECK(!overflow);
    return 1;
}

static int reset_client(void) {
    // Observe a fresh completion, rather than the previous Enabled title.
    SetWindowTextW(client, L"Typing - Transition requested");
    PostMessageW(client, WM_COMMAND, CLIENT_ENABLE, 0);
    CHECK(wait_status(L"Enabled"));
    return 1;
}

static int ordinary_input(void) {
    unsigned before = count(delivered, delivered_count, 'B', 0, 0);
    CHECK(tap('B'));
    CHECK(count(delivered, delivered_count, 'B', 0, 0) == before + 1);
    return 1;
}

static int wait_text(const wchar_t *expected) {
    ULONGLONG deadline = GetTickCount64() + 5000;
    while (character_count < wcslen(expected) && GetTickCount64() < deadline) pump(5);
    pump(5);
    if (wcscmp(characters, expected) != 0) {
        fwprintf(stderr, L"Text expected '%s', observed '%s'\n", expected, characters);
        return 0;
    }
    return 1;
}

static int toggle_caps_output(void) {
    unsigned down = count(delivered, delivered_count, VK_CAPITAL, 0, client_tag);
    unsigned up = count(delivered, delivered_count, VK_CAPITAL, 1, client_tag);
    CHECK(send_key(VK_CAPITAL, 0));
    CHECK(tap(VK_SPACE));
    CHECK(send_key(VK_CAPITAL, 1));
    CHECK(wait_output(VK_CAPITAL, down + 1, up + 1));
    return 1;
}

static int formatted_text(void) {
    SYSTEMTIME local;
    wchar_t date[32], time_text[128];
    int initially_on = GetKeyState(VK_CAPITAL) & 1;
    int caps;
    if (initially_on) CHECK(toggle_caps_output());
    for (caps = 0; caps <= 1; caps++) {
        GetLocalTime(&local);
        swprintf_s(date, ARRAYSIZE(date), L"%04u-%02u-%02u", local.wYear, local.wMonth, local.wDay);
        character_count = 0; characters[0] = 0;
        CHECK(tap('Y'));
        CHECK(wait_text(date));
        CHECK(GetTimeFormatEx(LOCALE_NAME_USER_DEFAULT, 0, &local, L"hh:mm tt", time_text, ARRAYSIZE(time_text)));
        character_count = 0; characters[0] = 0;
        CHECK(tap('T'));
        CHECK(wait_text(time_text));
        CHECK(!!(GetKeyState(VK_CAPITAL) & 1) == caps);
        CHECK(toggle_caps_output());
    }
    if (initially_on) CHECK(toggle_caps_output());
    puts("PASS actual client date/time output and Caps Lock restoration");
    return 1;
}
static int feedback_states(void) {
    POINT point;HDC screen;COLORREF top,bottom;
    CHECK(toggle_caps_output());pump(100);
    CHECK(GetPhysicalCursorPos(&point));screen=GetDC(NULL);
    top=GetPixel(screen,point.x+15,point.y+7);bottom=GetPixel(screen,point.x+15,point.y+22);ReleaseDC(NULL,screen);
    CHECK(top!=CLR_INVALID&&bottom!=CLR_INVALID);
    CHECK(GetBValue(top)>GetRValue(top)&&GetRValue(bottom)>GetBValue(bottom));
    CHECK(tap(VK_CAPITAL));pump(100);screen=GetDC(NULL);
    top=GetPixel(screen,point.x+15,point.y+7);bottom=GetPixel(screen,point.x+15,point.y+22);ReleaseDC(NULL,screen);
    CHECK(GetBValue(top)>GetRValue(top)&&GetBValue(bottom)>GetRValue(bottom));
    CHECK(tap(VK_CAPITAL));CHECK(toggle_caps_output());pump(100);screen=GetDC(NULL);
    top=GetPixel(screen,point.x+15,point.y+7);bottom=GetPixel(screen,point.x+15,point.y+22);ReleaseDC(NULL,screen);
    CHECK(GetRValue(top)>GetBValue(top)&&GetRValue(bottom)>GetBValue(bottom));
    CHECK(GetForegroundWindow()==receiver);
    puts("PASS rendered command-only, Caps-Lock-only, and combined half-circle feedback colors preserve focus");
    return 1;
}
static int normal_cases(void) {
    unsigned before, native_before;
    CHECK(ordinary_input());
    before = count(delivered, delivered_count, VK_CAPITAL, 0, 0);
    CHECK(tap(VK_CAPITAL));
    {
        ULONGLONG deadline=GetTickCount64()+2000;
        EnumWindows(find_client,0);
        while(!IsWindowVisible(overlay)&&GetTickCount64()<deadline)pump(5);
        CHECK(IsWindowVisible(overlay));
        CHECK((GetWindowLongPtrW(overlay,GWL_EXSTYLE)&(WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW))==
            (WS_EX_LAYERED|WS_EX_TRANSPARENT|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW));
        CHECK(SendMessageW(overlay,WM_NCHITTEST,0,0)==HTTRANSPARENT);
        CHECK(GetForegroundWindow()==receiver);
        puts("PASS command cursor overlay is visible, nonactivating, and pointer-transparent");
    }
    CHECK(count(delivered, delivered_count, VK_CAPITAL, 0, 0) == before);
    CHECK(count(delivered, delivered_count, VK_CAPITAL, 1, 0) == 1);
    CHECK(feedback_states());
    CHECK(send_key('J', 0));
    CHECK(send_key('J', 0));
    CHECK(send_key('J', 0));
    CHECK(send_key('J', 1));
    CHECK(wait_output(VK_LEFT, 3, 3));
    CHECK(count(delivered, delivered_count, 'J', 0, 0) == 0);
    CHECK(count(delivered, delivered_count, 'J', 1, 0) == 1);
    CHECK(count(delivered, delivered_count, VK_RIGHT, 0, client_tag) == 0);
    CHECK(tap(VK_LEFT));
    CHECK(wait_output(VK_RIGHT, 1, 1));
    CHECK(ordinary_input());
    CHECK(tap('Z'));
    CHECK(wait_output(VK_F12, 1, 1));
    CHECK(title_contains(L"Enabled"));
    puts("PASS own output matching the configured global hotkey does not toggle the client");
    puts("PASS command suppression, repeated J, own tag bypass, external injected binding");

    CHECK(tap(VK_CAPITAL));
    native_before = delivered_count;
    CHECK(tap(VK_LMENU));
    CHECK(count(delivered + native_before, delivered_count - native_before, VK_MENU, 0, 0) == 0);
    CHECK(count(delivered + native_before, delivered_count - native_before, VK_MENU, 1, 0) == 0);
    CHECK(tap('J'));
    CHECK(wait_output(VK_LEFT, 4, 4));
    puts("PASS Caps Lock and Left Alt mode gestures and native visibility");
    CHECK(formatted_text());

    CHECK(reset_client());
    before = count(delivered, delivered_count, VK_LEFT, 0, client_tag);
    CHECK(send_key(VK_CAPITAL, 0));
    CHECK(tap('J'));
    CHECK(send_key(VK_CAPITAL, 1));
    CHECK(wait_output(VK_LEFT, before + 1, before + 1));
    before = count(delivered, delivered_count, 'J', 0, 0);
    CHECK(tap('J'));
    CHECK(count(delivered, delivered_count, 'J', 0, 0) == before + 1);
    puts("PASS CapsLock+J shortcut leaves command mode off");

    CHECK(send_key(VK_LCONTROL, 0)); CHECK(send_key(VK_LSHIFT, 0)); CHECK(tap(VK_F12)); CHECK(send_key(VK_LSHIFT, 1)); CHECK(send_key(VK_LCONTROL, 1));
    CHECK(wait_status(L"Disabled"));
    CHECK(ordinary_input());
    CHECK(send_key(VK_LCONTROL, 0)); CHECK(send_key(VK_LSHIFT, 0)); CHECK(tap(VK_F12)); CHECK(send_key(VK_LSHIFT, 1)); CHECK(send_key(VK_LCONTROL, 1));
    CHECK(wait_status(L"Enabled"));
    puts("PASS configured Ctrl+Shift+F12 Disable and Enable with input passthrough");
    return 1;
}

static int recovery_cases(void) {
    unsigned before, ordinary;
    DWORD session=0;
    CHECK(reset_client());
    CHECK(send_key(VK_CAPITAL,0));
    SetWindowTextW(client,L"Typing - Missed suspend recovery requested");
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMEAUTOMATIC,0);
    CHECK(wait_status(L"Enabled"));
    CHECK(send_key(VK_CAPITAL,1));
    before=count(delivered,delivered_count,VK_LEFT,0,client_tag);
    ordinary=count(delivered,delivered_count,'J',0,0);
    CHECK(tap('J'));
    CHECK(count(delivered,delivered_count,'J',0,0)==ordinary+1);
    CHECK(count(delivered,delivered_count,VK_LEFT,0,client_tag)==before);
    CHECK(tap(VK_CAPITAL));
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMESUSPEND,0);
    pump(100);
    CHECK(tap('J'));
    CHECK(wait_output(VK_LEFT,before+1,before+1));
    puts("PASS missed suspend refreshes held-key baseline; paired user resume preserves new command mode");
    CHECK(ProcessIdToSessionId(client_pid,&session));
    PostMessageW(client,WM_WTSSESSION_CHANGE,7,session); /* lock */
    CHECK(wait_status(L"Paused"));CHECK(ordinary_input());
    PostMessageW(client,WM_COMMAND,CLIENT_DISABLE,0);
    PostMessageW(client,WM_WTSSESSION_CHANGE,8,session); /* unlock */
    CHECK(wait_status(L"Disabled"));CHECK(ordinary_input());
    CHECK(reset_client());
    PostMessageW(client,WM_WTSSESSION_CHANGE,4,session); /* remote disconnect */
    CHECK(wait_status(L"Paused"));
    PostMessageW(client,WM_WTSSESSION_CHANGE,3,session); /* remote connect */
    CHECK(wait_status(L"Enabled"));
    puts("PASS simulated lock/unlock and remote disconnect/connect preserve latest disabled/enabled target");
    CHECK(tap(VK_CAPITAL));CHECK(tap('V'));CHECK(tap('A'));
    interrupt_message=WM_POWERBROADCAST;interrupt_control=PBT_APMSUSPEND;
    CHECK(tap('J'));CHECK(wait_status(L"Paused"));
    CHECK(interrupt_control==0);
    CHECK(count(native_events,native_count,VK_RSHIFT,0,client_tag)==count(native_events,native_count,VK_RSHIFT,1,client_tag));
    CHECK(send_key(VK_LCONTROL,1));CHECK(ordinary_input());
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMEAUTOMATIC,0);
    CHECK(wait_status(L"Enabled"));
    CHECK(tap(VK_CAPITAL));
    before=count(delivered,delivered_count,VK_LEFT,0,client_tag);
    CHECK(tap('J'));CHECK(wait_output(VK_LEFT,before+1,before+1));
    interrupt_message=WM_COMMAND;
    puts("PASS simulated suspend cancels active selection output, releases Shift, and resumes command delivery");
    return 1;
}

/* Hold this disposable client's UI thread between Enable and its completion
   notification. Capture/output must not depend on timely tray presentation. */
static int hotkey_transition_case(void) {
    DWORD thread_id=GetWindowThreadProcessId(client,NULL);
    HANDLE ui_thread=OpenThread(THREAD_SUSPEND_RESUME,FALSE,thread_id);
    int staged=0,sent=0;unsigned native_before=0,delivered_before=0,early_native=0,early_delivered=0,early_passed=0;
    CHECK(ui_thread!=NULL);
    for(unsigned attempt=0;attempt<5&&!staged;attempt++) {
        PostMessageW(client,WM_COMMAND,CLIENT_DISABLE,0);
        if(!wait_status(L"Disabled")){CloseHandle(ui_thread);return 0;}
        SendMessageW(client,WM_COMMAND,CLIENT_ENABLE,0);
        if(SuspendThread(ui_thread)==(DWORD)-1){CloseHandle(ui_thread);return 0;}
        staged=title_contains(L"Disabled");
        if(staged) {
            pump(100);
            native_before=count(native_events,native_count,VK_F12,0,client_tag);
            delivered_before=count(delivered,delivered_count,VK_F12,0,client_tag);
            early_passed=count(delivered,delivered_count,VK_F10,0,0);
            sent=tap(VK_F10);pump(100);
            early_native=count(native_events,native_count,VK_F12,0,client_tag)-native_before;
            early_delivered=count(delivered,delivered_count,VK_F12,0,client_tag)-delivered_before;
            early_passed=count(delivered,delivered_count,VK_F10,0,0)-early_passed;
        }
        /* Always resume before any assertion or error return. */
        ResumeThread(ui_thread);pump(100);
    }
    CloseHandle(ui_thread);
    CHECK(staged&&sent);
    CHECK(early_native==early_delivered);
    CHECK(early_native==1||early_passed==1);
    CHECK(wait_status(L"Enabled"));
    CHECK(tap(VK_F10));CHECK(wait_output(VK_F12,delivered_before+early_delivered+1,delivered_before+early_delivered+1));
    CHECK(title_contains(L"Enabled"));
    puts("PASS generated global-hotkey chord is delivered without toggling while UI completion is delayed");
    return 1;
}

static BOOL CALLBACK find_warning(HWND window, LPARAM data) {
    DWORD pid;
    wchar_t title[128];
    GetWindowThreadProcessId(window, &pid);
    GetWindowTextW(window, title, ARRAYSIZE(title));
    if (pid == client_pid && wcscmp(title, L"Typing cleanup warning") == 0) {
        *(HWND *)data = window;
    }
    return TRUE;
}

typedef struct button_query { HWND button; unsigned count; } button_query;
static BOOL CALLBACK find_acknowledgment(HWND window, LPARAM data) {
    wchar_t name[128];
    button_query *result = (button_query *)data;
    GetClassNameW(window, name, ARRAYSIZE(name));
    if (_wcsicmp(name, L"Button") == 0) {
        result->button = window;
        result->count++;
    }
    return TRUE;
}
static int cleanup_case(UINT control, int reject_up) {
    unsigned ctrl_restores = count(native_events, native_count, VK_LCONTROL, 0, client_tag);
    CHECK(reset_client());
    CHECK(tap(VK_CAPITAL));
    CHECK(tap('V'));
    CHECK(tap('A'));
    interrupt_control = control;
    CHECK(tap('J'));
    if (reject_up && control == CLIENT_DISABLE) {
        CHECK(wait_status(L"Cleanup failed"));
        CHECK(count(native_events, native_count, VK_RSHIFT, 0, client_tag) >
              count(native_events, native_count, VK_RSHIFT, 1, client_tag));
        CHECK(count(native_events, native_count, VK_LCONTROL, 0, client_tag) > ctrl_restores);
        CHECK(send_key(VK_LCONTROL, 1));
        CHECK(ordinary_input());
        PostMessageW(client,WM_POWERBROADCAST,PBT_APMSUSPEND,0);
        PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMEAUTOMATIC,0);
        PostMessageW(client,WM_APP+5,0,0);
        pump(150);
        CHECK(count(native_events,native_count,VK_RSHIFT,0,client_tag)>
              count(native_events,native_count,VK_RSHIFT,1,client_tag));
        SetWindowTextW(client, L"Typing - Retry requested");
        PostMessageW(client, WM_COMMAND, CLIENT_DISABLE, 0);
        CHECK(wait_status(L"Disabled"));
        puts("PASS rejected release retained, admission closed, explicit Disable retry succeeds");
    } else if (reject_up) {
        HWND warning = NULL;
        ULONGLONG deadline = GetTickCount64() + 5000;
        while (!warning && GetTickCount64() < deadline) {
            EnumWindows(find_warning, (LPARAM)&warning);
            pump(5);
        }
        CHECK(warning != NULL);
        CHECK(count(native_events, native_count, VK_RSHIFT, 0, client_tag) >
              count(native_events, native_count, VK_RSHIFT, 1, client_tag));
        button_query acknowledgment = {0};
        EnumChildWindows(warning, find_acknowledgment, (LPARAM)&acknowledgment);
        CHECK(acknowledgment.count == 1);
        SendMessageW(acknowledgment.button, BM_CLICK, 0, 0);
        deadline = GetTickCount64() + 5000;
        while (WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT &&
               GetTickCount64() < deadline) pump(5);
        CHECK(WaitForSingleObject(client_process, 0) == WAIT_OBJECT_0);
        CHECK(focus_receiver(L"after Quit warning"));
        CHECK(send_key(VK_RSHIFT, 1));
        CHECK(send_key(VK_LCONTROL, 1));
        puts("PASS Quit warns about unresolved release and exits after acknowledgment");
        return 1;
    } else if (control == CLIENT_DISABLE) {
        CHECK(wait_status(L"Disabled"));
        CHECK(WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT);
        CHECK(count(native_events, native_count, VK_LCONTROL, 0, client_tag) > ctrl_restores);
        CHECK(send_key(VK_LCONTROL, 1));
        CHECK(ordinary_input());
    } else {
        ULONGLONG deadline = GetTickCount64() + 5000;
        while (WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT &&
               GetTickCount64() < deadline) pump(5);
        CHECK(WaitForSingleObject(client_process, 0) == WAIT_OBJECT_0);
    }
    if (driver_held[VK_LCONTROL]) CHECK(send_key(VK_LCONTROL, 1));
    CHECK(interrupt_control == 0);
    CHECK(count(native_events, native_count, VK_RSHIFT, 0, client_tag) ==
          count(native_events, native_count, VK_RSHIFT, 1, client_tag));
    puts(control == CLIENT_DISABLE ? "PASS Disable during active output releases Shift and keeps client running" :
                                    "PASS Quit during active output releases Shift and exits");
    return 1;
}

static int run(const wchar_t *host, const wchar_t *dll, const wchar_t *config,
               const wchar_t *log, const wchar_t *phase) {
    STARTUPINFOW startup = {0};
    PROCESS_INFORMATION process = {0};
    wchar_t command[4096];
    int ok = 0;
    CHECK(focus_receiver(L"before client launch"));
    startup.cb = sizeof(startup);
    startup.dwFlags = STARTF_USESHOWWINDOW;
    startup.wShowWindow = SW_HIDE;
    swprintf_s(command, ARRAYSIZE(command), L"\"%s\" --inputsim \"%s\" --config \"%s\" --log \"%s\" --state \"%s.state\"",
               host, dll, config, log, log);
    CHECK(CreateProcessW(host, command, NULL, NULL, FALSE, CREATE_NO_WINDOW, NULL, NULL, &startup, &process));
    client_process = process.hProcess;
    client_pid = process.dwProcessId;
    client_tag = sizeof(void*)==8 ? (ULONG_PTR)(UINT64_C(0x5459504900000000) | client_pid) : (ULONG_PTR)client_pid;
    if (!wait_status(L"Enabled")) { DWORD exit_code = 0; GetExitCodeProcess(client_process, &exit_code); fprintf(stderr, "Client exit/status code 0x%08lx\n", exit_code); goto cleanup; }
    if (IsWindowVisible(client)) { fprintf(stderr, "FAIL client startup must remain hidden\n"); goto cleanup; }
    if (!focus_receiver(L"after client startup")) goto cleanup;
    if (wcscmp(phase, L"hotkey-transition") == 0) ok = hotkey_transition_case();
    else if (wcscmp(phase, L"quit") == 0) ok = cleanup_case(CLIENT_QUIT, 0);
    else if (wcscmp(phase, L"failure-disable") == 0) ok = cleanup_case(CLIENT_DISABLE, 1);
    else if (wcscmp(phase, L"failure-quit") == 0) ok = cleanup_case(CLIENT_QUIT, 1);
    else ok = normal_cases() && recovery_cases() && cleanup_case(CLIENT_DISABLE, 0);
cleanup:
    if (WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT) {
        PostMessageW(client, WM_COMMAND, CLIENT_QUIT, 0);
        {
            ULONGLONG deadline = GetTickCount64() + 3000;
            while (WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT && GetTickCount64() < deadline) pump(5);
        }
        if (WaitForSingleObject(client_process, 0) == WAIT_TIMEOUT) {
            TerminateProcess(client_process, 2);
            ok = 0;
        }
    }
    if (!ok && count(native_events, native_count, VK_RSHIFT, 0, client_tag) >
               count(native_events, native_count, VK_RSHIFT, 1, client_tag)) {
        if (focus_receiver(L"release cleanup after failed case")) send_key(VK_RSHIFT, 1);
    }
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    client_process = NULL;
    return ok;
}

int wmain(int argc, wchar_t **argv) {
    WNDCLASSW cls = {0};
    HHOOK hook;
    POINT previous_cursor;
    HWND previous_window = GetForegroundWindow();
    int ok, initial_caps;
    UINT key;
    SetProcessDPIAware();
    if (argc != 6 && argc != 7) { fwprintf(stderr, L"Usage: windows_receiver HOST DLL CONFIG HOSTLOG PHASE [PROXY]\n"); return 2; }
    GetCursorPos(&previous_cursor);
    cls.lpfnWndProc = window_proc;
    cls.hInstance = GetModuleHandleW(NULL);
    cls.lpszClassName = L"TypingIntegrationReceiver";
    cls.hbrBackground=(HBRUSH)GetStockObject(WHITE_BRUSH);
    if (!RegisterClassW(&cls)) return 2;
    receiver = CreateWindowExW(0, cls.lpszClassName, L"Typing integration receiver",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE, 100, 100, 700, 400, NULL, NULL, cls.hInstance, NULL);
    if (!receiver) return 2;
    if (!wait_start_signal() || !focus_receiver(L"initial setup")) { DestroyWindow(receiver); return 2; }
    {POINT point={80,80};ClientToScreen(receiver,&point);SetCursorPos(point.x,point.y);}
    initial_caps = GetKeyState(VK_CAPITAL) & 1;
    for (key = 1; key <= 254; key++) {
        if (key == 1 || key == 2 || key == 4 || key == 5 || key == 6) continue;
        if (GetAsyncKeyState((int)key) & 0x8000) {
            fprintf(stderr, "FAIL keyboard key held before setup: %u\n", key);
            DestroyWindow(receiver);
            return 2;
        }
    }
    hook = SetWindowsHookExW(WH_KEYBOARD_LL, observe, cls.hInstance, 0);
    if (!hook) { DestroyWindow(receiver); return 2; }
    /* Feedback cases begin with command mode on and Caps Lock off. Preserve
       the user's initial toggle separately and restore it after all clients. */
    if (initial_caps && !tap(VK_CAPITAL)) {
        UnhookWindowsHookEx(hook);
        DestroyWindow(receiver);
        return 2;
    }
    delivered_count = native_count = character_count = 0;
    characters[0] = 0;
    overflow = 0;
    if (wcscmp(argv[5], L"all") == 0 && argc == 7) {
        const wchar_t *phases[] = {L"normal", L"quit", L"failure-disable", L"failure-quit", L"hotkey-transition"};
        unsigned phase;
        ok = 1;
        for (phase = 0; phase < ARRAYSIZE(phases) && ok; phase++) {
            wchar_t log_path[4096];
            const wchar_t *dll = phase == 2 || phase == 3 ? argv[6] : argv[2];
            client = NULL;
            delivered_count = native_count = character_count = 0;
            characters[0] = 0;
            overflow = 0;
            swprintf_s(log_path, ARRAYSIZE(log_path), L"%s-%s.log", argv[4], phases[phase]);
            ok = run(argv[1], dll, argv[3], log_path, phases[phase]);
            pump(10);
        }
    } else {
        ok = run(argv[1], argv[2], argv[3], argv[4], argv[5]);
    }
    if (focus_receiver(L"final keyboard cleanup")) {
        for (key = 1; key < 255; key++) {
            if (driver_held[key] && !send_key(key, 1)) ok = 0;
        }
        if (!!(GetKeyState(VK_CAPITAL) & 1) != !!initial_caps) {
            if (!tap(VK_CAPITAL)) ok = 0;
            if (!!(GetKeyState(VK_CAPITAL) & 1) != !!initial_caps) ok = 0;
        }
    } else {
        ok = 0;
    }
    UnhookWindowsHookEx(hook);
    if (GetForegroundWindow() == receiver && previous_window) SetForegroundWindow(previous_window);
    DestroyWindow(receiver);
    SetCursorPos(previous_cursor.x, previous_cursor.y);
    puts(ok ? "PASS Typing live integration" : "FAIL Typing live integration");
    return ok ? 0 : 1;
}
