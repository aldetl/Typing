#define UNICODE
#define _UNICODE
#include <windows.h>
#include <commctrl.h>
#include <stdio.h>
#include <wchar.h>
#include <string.h>
#include <stddef.h>

static HWND client;
static HANDLE process;
static DWORD client_pid;
static int failures;
#define CHECK(c) do { if(!(c)) { fwprintf(stderr,L"FAIL language UI line %d: %S (error %lu)\n",__LINE__,#c,GetLastError());failures++;goto cleanup; } } while(0)
static void pump(DWORD milliseconds) {
    ULONGLONG until=GetTickCount64()+milliseconds;
    do {MSG message;while(PeekMessageW(&message,NULL,0,0,PM_REMOVE)){TranslateMessage(&message);DispatchMessageW(&message);}
        if(GetTickCount64()>=until)break;MsgWaitForMultipleObjects(0,NULL,FALSE,5,QS_ALLINPUT);
    }while(1);
}
static BOOL CALLBACK find_client(HWND window,LPARAM unused) {
    DWORD pid=0;wchar_t name[128];(void)unused;
    GetWindowThreadProcessId(window,&pid);GetClassNameW(window,name,128);
    if(pid==client_pid&&wcscmp(name,L"TypingWindowsClient")==0)client=window;
    return TRUE;
}
static int wait_text(HWND window,const wchar_t *expected) {
    ULONGLONG until=GetTickCount64()+5000;wchar_t text[4096];
    do {SendMessageW(window,WM_GETTEXT,ARRAYSIZE(text),(LPARAM)text);if(wcsstr(text,expected))return 1;
        if(process&&WaitForSingleObject(process,0)!=WAIT_TIMEOUT)return 0;pump(10);
    }while(GetTickCount64()<until);return 0;
}
static HWND page(const wchar_t *title) {return FindWindowExW(client,NULL,L"TypingPage",title);}
static int tab(int index) {
    HWND tabs=GetDlgItem(client,2000);
    for(int tries=0;tries<8;tries++) {
        int current=(int)SendMessageW(tabs,TCM_GETCURSEL,0,0);if(current==index)return 1;
        SendMessageW(tabs,WM_KEYDOWN,current<index?VK_RIGHT:VK_LEFT,0);pump(20);
    }return 0;
}
static int choose(HWND settings,int index,const wchar_t *title) {
    HWND combo=GetDlgItem(settings,2020);
    SendMessageW(combo,CB_SETCURSEL,(WPARAM)index,0);
    SendMessageW(settings,WM_COMMAND,MAKEWPARAM(2020,CBN_SELCHANGE),(LPARAM)combo);
    return wait_text(settings,title);
}
static int state(HWND list,int index,UINT value,UINT mask) {
    LVITEMW item={0};SIZE_T bytes=0;item.state=value;item.stateMask=mask;
    void *remote=VirtualAllocEx(process,NULL,sizeof(item),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote)return 0;
    int ok=WriteProcessMemory(process,remote,&item,sizeof(item),&bytes)&&bytes==sizeof(item);
    if(ok)ok=(int)SendMessageW(list,LVM_SETITEMSTATE,(WPARAM)index,(LPARAM)remote);
    VirtualFreeEx(process,remote,0,MEM_RELEASE);return ok;
}
static int block_fits(HWND block) {
    RECT area={0};TEXTMETRICW metrics={0};
    HDC dc=GetDC(block);HGDIOBJ old=SelectObject(dc,(HFONT)SendMessageW(block,WM_GETFONT,0,0));
    int ok=GetTextMetricsW(dc,&metrics);SelectObject(dc,old);ReleaseDC(block,dc);
    SendMessageW(block,EM_GETRECT,0,(LPARAM)&area);
    LONG lines=(LONG)SendMessageW(block,EM_GETLINECOUNT,0,0);
    return ok&&lines>0&&area.bottom-area.top>=lines*metrics.tmHeight;
}
static int cell_is(HWND list,const wchar_t *expected) {
    struct cell {LVITEMW item;wchar_t text[128];} value={0};SIZE_T bytes=0;
    void *remote=VirtualAllocEx(process,NULL,sizeof(value),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote)return 0;
    value.item.pszText=(wchar_t*)((BYTE*)remote+offsetof(struct cell,text));value.item.cchTextMax=ARRAYSIZE(value.text);
    int ok=WriteProcessMemory(process,remote,&value,sizeof(value),&bytes)&&bytes==sizeof(value);
    if(ok){SendMessageW(list,LVM_GETITEMTEXTW,0,(LPARAM)remote);ok=ReadProcessMemory(process,remote,&value,sizeof(value),&bytes)&&bytes==sizeof(value)&&wcscmp(value.text,expected)==0;}
    VirtualFreeEx(process,remote,0,MEM_RELEASE);return ok;
}
static int write_text(const wchar_t *path,const char *text) {
    FILE *file=NULL;if(_wfopen_s(&file,path,L"wb")||!file)return 0;
    int ok=fputs(text,file)>=0;return fclose(file)==0&&ok;
}
static int selection_is(const wchar_t *path,const char *expected) {
    FILE *file=NULL;char text[64]={0};if(_wfopen_s(&file,path,L"rb")||!file)return 0;
    size_t size=fread(text,1,sizeof(text)-1,file);fclose(file);return size>0&&strcmp(text,expected)==0;
}
static int start_process(const wchar_t *application,const wchar_t *directory) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION started={0};wchar_t command[8192];startup.cb=sizeof(startup);
    swprintf_s(command,ARRAYSIZE(command),L"\"%s\" --log \"%s\\typing.log\" --state \"%s\\window.txt\"",application,directory,directory);
    if(!CreateProcessW(application,command,NULL,NULL,FALSE,0,NULL,directory,&startup,&started))return 0;
    process=started.hProcess;client_pid=started.dwProcessId;client=NULL;CloseHandle(started.hThread);
    return 1;
}
static int launch(const wchar_t *application,const wchar_t *directory) {
    if(!start_process(application,directory))return 0;
    ULONGLONG until=GetTickCount64()+5000;
    do {EnumWindows(find_client,0);if(client)return 1;pump(10);}while(GetTickCount64()<until&&WaitForSingleObject(process,0)==WAIT_TIMEOUT);
    return 0;
}
static int stop(void) {
    if(client)PostMessageW(client,WM_COMMAND,1003,0);
    int ok=WaitForSingleObject(process,5000)==WAIT_OBJECT_0;DWORD code=99;
    ok=GetExitCodeProcess(process,&code)&&code==0&&ok;CloseHandle(process);process=NULL;client=NULL;return ok;
}
static HWND guide_dialog(void) {
    ULONGLONG until=GetTickCount64()+5000;
    do {HWND dialog=FindWindowW(L"TypingGuideDialog",NULL);DWORD pid=0;GetWindowThreadProcessId(dialog,&pid);
        if(dialog&&pid==client_pid)return dialog;pump(10);
    }while(GetTickCount64()<until);return NULL;
}
static BOOL CALLBACK find_startup_error(HWND window,LPARAM result) {
    DWORD pid=0;wchar_t title[128];GetWindowThreadProcessId(window,&pid);
    if(pid==client_pid&&GetWindowTextW(window,title,ARRAYSIZE(title))&&wcscmp(title,L"Typing \u542f\u52a8\u9519\u8bef")==0&&IsWindowVisible(window)) {
        *(HWND*)result=window;return FALSE;
    }
    return TRUE;
}
static HWND startup_error_dialog(void) {
    ULONGLONG until=GetTickCount64()+5000;
    do {HWND dialog=NULL;EnumWindows(find_startup_error,(LPARAM)&dialog);if(dialog)return dialog;pump(10);}while(GetTickCount64()<until);
    return NULL;
}
int wmain(int argc,wchar_t **argv) {
    wchar_t application[4096],dll[4096],profiles_dir[4096],profile[4096],config[4096],languages[4096],source[4096],destination[4096],user[4096],user_typing[4096],preference[4096];
    HWND settings=NULL,mappings=NULL,list=NULL,help=NULL,guide=NULL,events=NULL;LRESULT event_count=0;
    if(argc!=5){fputs("usage: language_ui APP DLL FIXTURE_DIRECTORY LANGUAGE_SOURCE_DIRECTORY\n",stderr);return 2;}
    CHECK(CreateDirectoryW(argv[3],NULL)||GetLastError()==ERROR_ALREADY_EXISTS);
    swprintf_s(application,ARRAYSIZE(application),L"%s\\Typing.exe",argv[3]);CHECK(CopyFileW(argv[1],application,FALSE));
    swprintf_s(dll,ARRAYSIZE(dll),L"%s\\inputsim.dll",argv[3]);CHECK(CopyFileW(argv[2],dll,FALSE));
    swprintf_s(profiles_dir,ARRAYSIZE(profiles_dir),L"%s\\profiles",argv[3]);CHECK(CreateDirectoryW(profiles_dir,NULL));
    swprintf_s(profile,ARRAYSIZE(profile),L"%s\\default.typing.ini",profiles_dir);
    CHECK(write_text(profile,"[profile]\nversion=1\nname=Language fixture\n[modekeys]\nCapsLock=mode\nJ=Left\n[hotkeys]\n"));
    swprintf_s(config,ARRAYSIZE(config),L"%s\\configuration.ini",argv[3]);CHECK(write_text(config,"[common]\nhotkey=Control+Shift+F12\n[profiles]\nactive=profiles/default.typing.ini\n"));
    swprintf_s(languages,ARRAYSIZE(languages),L"%s\\languages",argv[3]);CHECK(CreateDirectoryW(languages,NULL));
    {const wchar_t *files[]={L"en.ini",L"zh-CN.ini",L"zh-CN.md"};for(unsigned i=0;i<ARRAYSIZE(files);i++){
        swprintf_s(source,ARRAYSIZE(source),L"%s\\%s",argv[4],files[i]);swprintf_s(destination,ARRAYSIZE(destination),L"%s\\%s",languages,files[i]);CHECK(CopyFileW(source,destination,FALSE));}}
    swprintf_s(user,ARRAYSIZE(user),L"%s\\user",argv[3]);CHECK(CreateDirectoryW(user,NULL));
    swprintf_s(user_typing,ARRAYSIZE(user_typing),L"%s\\Typing",user);CHECK(CreateDirectoryW(user_typing,NULL));
    swprintf_s(preference,ARRAYSIZE(preference),L"%s\\language.txt",user_typing);CHECK(write_text(preference,"en\n"));
    CHECK(SetEnvironmentVariableW(L"LOCALAPPDATA",user));
    CHECK(launch(application,argv[3]));CHECK(wait_text(client,L"Enabled"));CHECK(!IsWindowVisible(client));
    PostMessageW(client,WM_COMMAND,1004,0);pump(80);CHECK(IsWindowVisible(client));
    settings=page(L"Settings");mappings=page(L"Mapping Profile");help=page(L"Help");CHECK(settings&&mappings&&help);
    list=FindWindowExW(mappings,NULL,L"SysListView32",NULL);CHECK(list);
    CHECK(state(list,1,LVIS_SELECTED|LVIS_FOCUSED,LVIS_SELECTED|LVIS_FOCUSED));
    CHECK(state(list,1,1u<<12,LVIS_STATEIMAGEMASK));
    CHECK(wait_text(FindWindowExW(mappings,NULL,L"EDIT",NULL),L"Unsaved changes"));
    CHECK(tab(1));events=FindWindowExW(page(L"Event diagnostics"),NULL,L"SysListView32",NULL);CHECK(events);
    SetForegroundWindow(client);
    if(GetForegroundWindow()==client) {
        INPUT input[2]={0};input[0].type=INPUT_KEYBOARD;input[0].ki.wVk=VK_F9;input[1]=input[0];input[1].ki.dwFlags=KEYEVENTF_KEYUP;
        CHECK(SendInput(2,input,sizeof(INPUT))==2);
        pump(120);event_count=SendMessageW(events,LVM_GETITEMCOUNT,0,0);CHECK(event_count>=2);
    } else puts("SKIP live event-history input: fixture was not foreground; no input sent");
    CHECK(tab(2));CHECK(choose(settings,2,L"\u8bbe\u7f6e"));
    CHECK(wait_text(GetDlgItem(mappings,3002),L"\u4fdd\u5b58"));
    CHECK(wait_text(list,L"\u914d\u7f6e\u4e2d\u7684\u6309\u952e\u6620\u5c04"));
    CHECK(wait_text(FindWindowExW(mappings,NULL,L"EDIT",NULL),L"\u6709\u672a\u4fdd\u5b58\u7684\u66f4\u6539"));
    CHECK(SendMessageW(list,LVM_GETNEXTITEM,(WPARAM)-1,LVNI_SELECTED)==1);
    CHECK((SendMessageW(list,LVM_GETITEMSTATE,1,LVIS_STATEIMAGEMASK)>>12)==1);
    CHECK(selection_is(preference,"zh-CN\n"));
    if(event_count) {
        CHECK(SendMessageW(events,LVM_GETITEMCOUNT,0,0)==event_count);CHECK(cell_is(events,L"\u91ca\u653e"));
        puts("PASS diagnostic rows retain their history and translate after switching");
    }
    {const UINT scales[]={96,120,144,192};
        for(unsigned i=0;i<ARRAYSIZE(scales);i++) {
            RECT combo_bounds,checkbox_bounds,label_bounds;
            SendMessageW(client,WM_DPICHANGED,MAKELONG(scales[i],scales[i]),0);
            SetWindowPos(client,NULL,0,0,420,430,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE);pump(20);
            CHECK(GetWindowRect(GetDlgItem(settings,2020),&combo_bounds));
            CHECK(GetWindowRect(GetDlgItem(settings,2019),&checkbox_bounds));
            CHECK(GetWindowRect(GetDlgItem(settings,2021),&label_bounds));
            CHECK(combo_bounds.bottom<=checkbox_bounds.top&&label_bounds.bottom<=checkbox_bounds.top);
            CHECK(block_fits(FindWindowExW(settings,NULL,L"EDIT",NULL)));
            CHECK(wait_text(GetDlgItem(mappings,3002),L"\u4fdd\u5b58"));
        }
    }
    CHECK(tab(3));PostMessageW(client,WM_COMMAND,2012,0);guide=guide_dialog();CHECK(guide);
    CHECK(wait_text(guide,L"Typing \u7528\u6237\u6307\u5357"));
    CHECK(wait_text(FindWindowExW(guide,NULL,L"RICHEDIT50W",NULL),L"\u5f00\u59cb\u4f7f\u7528"));
    PostMessageW(guide,WM_CLOSE,0,0);pump(50);guide=NULL;
    CHECK(tab(2));CHECK(choose(settings,1,L"Settings"));CHECK(selection_is(preference,"en\n"));
    CHECK(wait_text(GetDlgItem(settings,2019),L"Start with &Windows"));
    swprintf_s(destination,ARRAYSIZE(destination),L"%s\\zh-CN.ini",languages);CHECK(write_text(destination,"[language]\nversion=999\n"));
    CHECK(choose(settings,2,L"Settings"));CHECK(wait_text(GetDlgItem(settings,2022),L"Invalid language profile"));
    CHECK(block_fits(GetDlgItem(settings,2022)));
    CHECK(SendMessageW(GetDlgItem(settings,2020),CB_GETCURSEL,0,0)==1);CHECK(selection_is(preference,"en\n"));
    swprintf_s(source,ARRAYSIZE(source),L"%s\\zh-CN.ini",argv[4]);CHECK(CopyFileW(source,destination,FALSE));
    CHECK(SetFileAttributesW(preference,FILE_ATTRIBUTE_READONLY));CHECK(choose(settings,2,L"Settings"));
    CHECK(wait_text(GetDlgItem(settings,2022),L"Cannot save language preference"));CHECK(selection_is(preference,"en\n"));
    CHECK(SetFileAttributesW(preference,FILE_ATTRIBUTE_NORMAL));
    CHECK(choose(settings,2,L"\u8bbe\u7f6e"));
    // Save only the disposable draft, allowing a normal quit without a prompt.
    CHECK(tab(0));SendMessageW(mappings,WM_COMMAND,3002,0);pump(60);CHECK(stop());
    CHECK(launch(application,argv[3]));CHECK(wait_text(client,L"Typing - "));CHECK(!IsWindowVisible(client));
    PostMessageW(client,WM_COMMAND,1004,0);pump(80);CHECK(page(L"\u8bbe\u7f6e"));
    CHECK(stop());
    CHECK(DeleteFileW(destination));
    CHECK(launch(application,argv[3]));CHECK(wait_text(client,L"Enabled"));
    PostMessageW(client,WM_COMMAND,1004,0);pump(80);settings=page(L"Settings");CHECK(settings);
    CHECK(wait_text(GetDlgItem(settings,2022),L"Saved language zh-CN is unavailable"));CHECK(selection_is(preference,"zh-CN\n"));CHECK(stop());
    // The language must be available before application configuration is read.
    CHECK(CopyFileW(source,destination,FALSE));
    {const char *invalid="[common]\nmode_auto_exit=invalid\n";
        CHECK(write_text(config,invalid));CHECK(start_process(application,argv[3]));
        guide=startup_error_dialog();CHECK(guide);EnumWindows(find_client,0);CHECK(!client);
        HWND detail=NULL;wchar_t body[4096];int found=0;
        while((detail=FindWindowExW(guide,detail,L"STATIC",NULL))!=NULL) {
            if(GetWindowTextW(detail,body,ARRAYSIZE(body))&&wcsstr(body,L"\u914d\u7f6e")&&wcsstr(body,L"configuration.ini"))found=1;
        }
        CHECK(found);SendMessageW(guide,WM_CLOSE,0,0);guide=NULL;
        DWORD code=99;CHECK(WaitForSingleObject(process,5000)==WAIT_OBJECT_0&&GetExitCodeProcess(process,&code)&&code==1);
        CloseHandle(process);process=NULL;CHECK(selection_is(config,invalid));
    }
    puts("PASS saved Chinese language applies to a fatal configuration error before the main window exists");
    puts("PASS live English/Chinese switch, translated guide, preserved dirty draft/selection/check state, narrow simulated-DPI layout, invalid-file and preference-write failures, restart persistence and startup fallback");
cleanup:
    if(guide)PostMessageW(guide,WM_CLOSE,0,0);
    if(process){if(client)PostMessageW(client,WM_COMMAND,1003,0);if(WaitForSingleObject(process,1000)!=WAIT_OBJECT_0)TerminateProcess(process,1);CloseHandle(process);}
    return failures?1:0;
}
