#define UNICODE
#define _UNICODE
#include <windows.h>
#include <commctrl.h>
#include <richedit.h>
#include <stdio.h>
#include <wchar.h>
#include <string.h>

static HWND client;
static DWORD client_pid;
static HANDLE process;
static int failures;
#define CHECK(c) do { if (!(c)) { fwprintf(stderr,L"FAIL UI line %d: %S (error %lu)\n",__LINE__,#c,GetLastError()); failures++; goto cleanup; } } while(0)
static BOOL CALLBACK find_client(HWND window,LPARAM unused) {
    wchar_t name[128];DWORD pid=0;(void)unused;
    GetWindowThreadProcessId(window,&pid);GetClassNameW(window,name,128);
    if(pid==client_pid&&wcscmp(name,L"TypingWindowsClient")==0)client=window;
    return TRUE;
}
static void pump(DWORD milliseconds) {
    ULONGLONG until=GetTickCount64()+milliseconds;
    do {MSG message;while(PeekMessageW(&message,NULL,0,0,PM_REMOVE)){TranslateMessage(&message);DispatchMessageW(&message);}
        if(GetTickCount64()>=until)break;MsgWaitForMultipleObjects(0,NULL,FALSE,5,QS_ALLINPUT);
    } while(1);
}
static int status(const wchar_t *text) {
    ULONGLONG until=GetTickCount64()+5000;wchar_t title[512];
    do {if(!client)EnumWindows(find_client,0);GetWindowTextW(client,title,512);
        if(wcsstr(title,text))return 1;if(WaitForSingleObject(process,0)!=WAIT_TIMEOUT)break;pump(10);
    } while(GetTickCount64()<until);return 0;
}
static HWND page(const wchar_t *name) {
    HWND child=NULL;while((child=FindWindowExW(client,child,L"TypingPage",NULL))!=NULL){
        wchar_t title[128];GetWindowTextW(child,title,128);if(wcscmp(title,name)==0)return child;
    }return NULL;
}
static int block_fits(HWND block) {
    RECT area={0};TEXTMETRICW metrics={0};LOGFONTW logical={0};
    if(GetObjectW((HFONT)SendMessageW(block,WM_GETFONT,0,0),sizeof(logical),&logical)!=sizeof(logical))return 0;
    HFONT font=CreateFontIndirectW(&logical);if(!font)return 0;
    HDC dc=GetDC(block);HGDIOBJ old=SelectObject(dc,font);
    int ok=GetTextMetricsW(dc,&metrics);SelectObject(dc,old);ReleaseDC(block,dc);DeleteObject(font);
    SendMessageW(block,EM_GETRECT,0,(LPARAM)&area);
    LONG lines=(LONG)SendMessageW(block,EM_GETLINECOUNT,0,0);
    return ok&&lines>0&&area.bottom-area.top>=lines*metrics.tmHeight&&
        SendMessageW(block,EM_GETFIRSTVISIBLELINE,0,0)==0;
}
static int visible(int expected) {
    ULONGLONG until=GetTickCount64()+5000;
    while(GetTickCount64()<until&&WaitForSingleObject(process,0)==WAIT_TIMEOUT) {
        if(!!IsWindowVisible(client)==expected)return 1;pump(10);
    }return 0;
}
static BOOL CALLBACK find_about(HWND window,LPARAM output) {
    DWORD pid=0;wchar_t title[128];GetWindowThreadProcessId(window,&pid);GetWindowTextW(window,title,128);
    if(pid==client_pid&&wcscmp(title,L"About Typing")==0)*(HWND*)output=window;
    return TRUE;
}
static HWND about_content(HWND dialog) {
    HWND text=NULL;wchar_t content[2048];
    while((text=FindWindowExW(dialog,text,L"STATIC",NULL))!=NULL) {
        if(GetWindowTextW(text,content,ARRAYSIZE(content))>0&&wcsstr(content,L"Typing "))return text;
    }
    return NULL;
}
static HWND wait_about(void) {
    HWND dialog=NULL;ULONGLONG until=GetTickCount64()+5000;
    do {EnumWindows(find_about,(LPARAM)&dialog);if(dialog&&IsWindowVisible(dialog)&&about_content(dialog))return dialog;pump(10);}while(GetTickCount64()<until);
    return NULL;
}
static BOOL CALLBACK find_guide(HWND window,LPARAM output) {
    DWORD pid=0;wchar_t title[128];GetWindowThreadProcessId(window,&pid);GetWindowTextW(window,title,128);
    if(pid==client_pid&&wcscmp(title,L"Typing User Guide")==0)*(HWND*)output=window;
    return TRUE;
}
static HWND wait_guide(void) {
    HWND dialog=NULL;ULONGLONG until=GetTickCount64()+5000;
    /* ShowWindow precedes activation; BM_CLICK requires an active dialog. */
    do {EnumWindows(find_guide,(LPARAM)&dialog);
        if(dialog&&IsWindowVisible(dialog)&&GetForegroundWindow()==dialog&&
            GetDlgItem(dialog,2011)&&IsWindowEnabled(GetDlgItem(dialog,2)))return dialog;
        pump(10);
    }while(GetTickCount64()<until);
    return NULL;
}
static int dismissed(HWND dialog) {
    ULONGLONG until=GetTickCount64()+5000;
    while(IsWindow(dialog)&&GetTickCount64()<until)pump(10);
    return !IsWindow(dialog)&&IsWindowEnabled(client);
}
static void make_command(wchar_t *command,wchar_t **argv) {
    wchar_t resources[4096];resources[0]=0;
    if(wcscmp(argv[2],L"-")!=0&&wcscmp(argv[3],L"-")!=0)
        swprintf_s(resources,4096,L"--inputsim \"%s\" --config \"%s\"",argv[2],argv[3]);
    swprintf_s(command,8192,L"\"%s\" %s --log \"%s\" --state \"%s.state\"",argv[1],resources,argv[4],argv[4]);
}
static int preference_boundary(wchar_t **argv,int unwritable) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION started={0};
    wchar_t command[8192],base[8192],state_path[4096];
    DWORD code=99;int ok=0;FILE *file=NULL;char saved[128]={0};size_t size;
    swprintf_s(state_path,ARRAYSIZE(state_path),L"%s.%s-state",argv[4],unwritable?L"directory":L"invalid");
    if(unwritable) {if(!CreateDirectoryW(state_path,NULL)&&GetLastError()!=ERROR_ALREADY_EXISTS)return 0;}
    else {if(_wfopen_s(&file,state_path,L"wb")||!file)return 0;fputs("invalid preference",file);fclose(file);}
    make_command(base,argv);swprintf_s(command,ARRAYSIZE(command),L"%s --state \"%s\"",base,state_path);
    startup.cb=sizeof(startup);startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
    client=NULL;
    if(!CreateProcessW(argv[1],command,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,NULL,&startup,&started))return 0;
    process=started.hProcess;client_pid=started.dwProcessId;
    if(!status(L"Enabled")||IsWindowVisible(client))goto cleanup;
    if(unwritable) {
        RECT rect;
        PostMessageW(client,WM_COMMAND,1004,0);if(!visible(1)||!GetWindowRect(client,&rect))goto cleanup;
        SendMessageW(client,WM_ENTERSIZEMOVE,0,0);SendMessageW(client,WM_SIZING,WMSZ_BOTTOMRIGHT,(LPARAM)&rect);
        if(!SetWindowPos(client,NULL,0,0,rect.right-rect.left+20,rect.bottom-rect.top+20,SWP_NOMOVE|SWP_NOZORDER))goto cleanup;
        SendMessageW(client,WM_EXITSIZEMOVE,0,0); /* Saving to a directory must not block the UI. */
        PostMessageW(client,WM_CLOSE,0,0);if(!visible(0))goto cleanup;
    }
    ok=1;
cleanup:
    if(client)PostMessageW(client,WM_COMMAND,1003,0);
    if(WaitForSingleObject(process,5000)!=WAIT_OBJECT_0)ok=0;
    if(!GetExitCodeProcess(process,&code)||code!=0)ok=0;
    CloseHandle(process);CloseHandle(started.hThread);client=NULL;process=NULL;
    if(!unwritable) {
        if(_wfopen_s(&file,state_path,L"rb")||!file)return 0;
        size=fread(saved,1,sizeof(saved)-1,file);fclose(file);
        if(size!=strlen("invalid preference")||strcmp(saved,"invalid preference")!=0)ok=0;
    }
    if(ok)puts(unwritable?"PASS unwritable preference does not block resize, hide, or Exit":"PASS hidden startup/Exit leaves invalid size preference unchanged");
    return ok;
}
static int logging_boundary(wchar_t **argv) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION started={0};
    wchar_t base[8192],command[16384],directory[4096];DWORD code=99;int ok=0;
    swprintf_s(directory,ARRAYSIZE(directory),L"%s.unusable-log",argv[4]);
    if(!CreateDirectoryW(directory,NULL)&&GetLastError()!=ERROR_ALREADY_EXISTS)return 0;
    make_command(base,argv);
    swprintf_s(command,ARRAYSIZE(command),L"%s --log \"%s\"",base,directory);
    startup.cb=sizeof(startup);startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
    client=NULL;
    if(!CreateProcessW(argv[1],command,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,NULL,&startup,&started))return 0;
    process=started.hProcess;client_pid=started.dwProcessId;
    if(!status(L"Enabled")||IsWindowVisible(client))goto cleanup;
    PostMessageW(client,WM_COMMAND,1004,0);if(!visible(1))goto cleanup;
    PostMessageW(client,WM_COMMAND,1002,0);if(!status(L"Disabled"))goto cleanup;
    PostMessageW(client,WM_COMMAND,1001,0);if(!status(L"Enabled"))goto cleanup;
    ok=1;
cleanup:
    if(client)PostMessageW(client,WM_COMMAND,1003,0);
    if(WaitForSingleObject(process,5000)!=WAIT_OBJECT_0)ok=0;
    if(!GetExitCodeProcess(process,&code)||code!=0)ok=0;
    CloseHandle(process);CloseHandle(started.hThread);process=NULL;client=NULL;
    if(ok)puts("PASS unusable log destination is silent and does not block startup, controls, or Exit");
    return ok;
}
static int restart_check(wchar_t **argv,RECT expected) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION started={0};wchar_t command[8192];
    RECT actual;FILETIME created,exited,kernel_before,user_before,kernel_after,user_after;
    ULARGE_INTEGER kb,ub,ka,ua;int ok=0;
    make_command(command,argv);startup.cb=sizeof(startup);
    startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
    client=NULL;
    if(!CreateProcessW(argv[1],command,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,NULL,&startup,&started))return 0;
    process=started.hProcess;client_pid=started.dwProcessId;
    if(!status(L"Enabled")||IsWindowVisible(client))goto cleanup;
    if(!GetProcessTimes(process,&created,&exited,&kernel_before,&user_before))goto cleanup;
    pump(2000);
    if(!GetProcessTimes(process,&created,&exited,&kernel_after,&user_after))goto cleanup;
    kb.LowPart=kernel_before.dwLowDateTime;kb.HighPart=kernel_before.dwHighDateTime;
    ub.LowPart=user_before.dwLowDateTime;ub.HighPart=user_before.dwHighDateTime;
    ka.LowPart=kernel_after.dwLowDateTime;ka.HighPart=kernel_after.dwHighDateTime;
    ua.LowPart=user_after.dwLowDateTime;ua.HighPart=user_after.dwHighDateTime;
    printf("INFO hidden enabled idle CPU: %.2f ms over 2 seconds (short sample)\n",
        (double)(ka.QuadPart-kb.QuadPart+ua.QuadPart-ub.QuadPart)/10000.0);
    PostMessageW(client,WM_COMMAND,1004,0);
    if(!visible(1)||!GetClientRect(client,&actual))goto cleanup;
    printf("INFO restart client size %ld x %ld at %u DPI; expected %ld x %ld\n",
        actual.right,actual.bottom,GetDpiForWindow(client),expected.right,expected.bottom);
    // Preferences use integer logical pixels. A resize at fractional scaling
    // can quantize by up to one logical pixel on the next process start.
    {LONG quantum=(LONG)((GetDpiForWindow(client)+95)/96);
        if(labs(actual.right-expected.right)>quantum||labs(actual.bottom-expected.bottom)>quantum)goto cleanup;}
    puts("PASS UI persisted normal size restored across process restart");ok=1;
cleanup:
    if(client)PostMessageW(client,WM_COMMAND,1003,0);
    if(WaitForSingleObject(process,5000)!=WAIT_OBJECT_0)ok=0;
    {DWORD code=99;if(!GetExitCodeProcess(process,&code)||code!=0)ok=0;}
    CloseHandle(process);CloseHandle(started.hThread);process=NULL;client=NULL;
    return ok;
}
int wmain(int argc,wchar_t **argv) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION started={0};wchar_t command[8192];
    HWND about,guide,open_guide,introduction,dialog=NULL;RECT rect={0},outer;DWORD exit_code=99;
    int inspect=0;
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    if(argc==3&&wcscmp(argv[1],L"--quit")==0) {
        client_pid=wcstoul(argv[2],NULL,10);process=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,client_pid);
        if(!process)return 2;EnumWindows(find_client,0);
        if(!client||!PostMessageW(client,WM_COMMAND,1003,0)||WaitForSingleObject(process,5000)!=WAIT_OBJECT_0){CloseHandle(process);return 1;}
        {DWORD code=99;GetExitCodeProcess(process,&code);CloseHandle(process);return code==0?0:1;}
    } else if(argc==4&&wcscmp(argv[1],L"--attach")==0) {
        client_pid=wcstoul(argv[2],NULL,10);process=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,client_pid);
        inspect=wcscmp(argv[3],L"inspect")==0;
    } else if(argc==6) {
        startup.cb=sizeof(startup);
        startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
        make_command(command,argv);
        if(!CreateProcessW(argv[1],command,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,NULL,&startup,&started))return 2;
        process=started.hProcess;client_pid=started.dwProcessId;inspect=wcscmp(argv[5],L"inspect")==0;
    } else {fwprintf(stderr,L"Usage: native_ui HOST DLL CONFIG LOG inspect|test\n       native_ui --attach PID inspect|test\n");return 2;}
    CHECK(process!=NULL);CHECK(status(L"Enabled"));CHECK(!IsWindowVisible(client));
    CHECK(GetDpiForWindow(client)>0);
    puts("PASS UI hidden startup and native window");fflush(stdout);
    CHECK(PostMessageW(client,WM_COMMAND,1004,0));CHECK(visible(1));
    CHECK(GetClientRect(client,&rect));CHECK(rect.right>=300&&rect.bottom>=200);
    CHECK(GetDlgItem(client,2000)!=NULL);CHECK(SendMessageW(GetDlgItem(client,2000),TCM_GETITEMCOUNT,0,0)==4);
    CHECK(FindWindowExW(client,NULL,L"STATIC",NULL)==NULL);
    CHECK(page(L"Command")==NULL);
    about=GetDlgItem(page(L"Help"),2007);open_guide=GetDlgItem(page(L"Help"),2012);
    introduction=GetDlgItem(page(L"Help"),2013);CHECK(about&&open_guide&&introduction);
    CHECK(GetDlgItem(page(L"Help"),2011)==NULL);
    {const wchar_t *expected[]={L"move the caret quickly",L"Default profile",L"\x2022 Vim style:",L"\x2022 Emacs style:",L"\x2022 Key mapping:",L"Ctrl+Left"};
        wchar_t content[2048];CHECK(GetWindowTextW(introduction,content,2048)>0);
        for(unsigned i=0;i<ARRAYSIZE(expected);i++) {
            CHECK(wcsstr(content,expected[i])!=NULL);
        }
        CHECK(wcsstr(content,L"Default profile:\r\n    \x2022 Vim style:")!=NULL);
        CHECK(wcsstr(content,L"\r\n\r\n    \x2022")==NULL);
        CHECK(GetDlgItem(page(L"Help"),2014)==NULL);
        CHECK((GetWindowLongW(introduction,GWL_STYLE)&(ES_READONLY|ES_MULTILINE))==(ES_READONLY|ES_MULTILINE));
        SendMessageW(introduction,EM_SETSEL,0,-1);
        CHECK(HIWORD(SendMessageW(introduction,EM_GETSEL,0,0))==(WORD)wcslen(content));
        SendMessageW(introduction,EM_SETSEL,0,0);}
    CHECK(page(L"Help")&&page(L"Event diagnostics")&&page(L"Settings")&&page(L"Mapping Profile"));
    {HWND checkbox=GetDlgItem(page(L"Settings"),2019);wchar_t label[128];
        CHECK(checkbox&&GetWindowTextW(checkbox,label,128)>0);
        CHECK(wcscmp(label,L"Start with &Windows")==0);
        CHECK((GetWindowLongW(checkbox,GWL_STYLE)&BS_TYPEMASK)==BS_CHECKBOX);
        CHECK((GetWindowLongW(checkbox,GWL_STYLE)&WS_TABSTOP)!=0);}
    {HWND settings=page(L"Settings"),explanation=FindWindowExW(settings,NULL,L"EDIT",NULL);
        HWND error=FindWindowExW(settings,explanation,L"EDIT",NULL);wchar_t content[1024];RECT original,wide,narrow;
        HWND language_error=GetDlgItem(settings,2022);
        CHECK(explanation&&error&&language_error&&GetDlgItem(settings,2020));
        CHECK(FindWindowExW(settings,error,L"EDIT",NULL)==language_error&&FindWindowExW(settings,language_error,L"EDIT",NULL)==NULL);
        CHECK((GetWindowLongW(error,GWL_STYLE)&WS_VISIBLE)==0);
        CHECK(FindWindowExW(settings,NULL,L"STATIC",NULL)==GetDlgItem(settings,2021));
        CHECK((GetWindowLongW(GetDlgItem(settings,2020),GWL_STYLE)&(CBS_DROPDOWNLIST|WS_TABSTOP))==(CBS_DROPDOWNLIST|WS_TABSTOP));
        CHECK(GetWindowTextW(explanation,content,ARRAYSIZE(content))>0);
        CHECK(wcsstr(content,L"system tray")&&wcsstr(content,L"new location"));
        CHECK((GetWindowLongW(explanation,GWL_STYLE)&(ES_READONLY|ES_MULTILINE))==(ES_READONLY|ES_MULTILINE));
        CHECK(GetWindowRect(client,&original)&&GetWindowRect(explanation,&wide)&&block_fits(explanation));
        CHECK(SetWindowPos(client,NULL,0,0,MulDiv(360,(int)GetDpiForWindow(client),96),original.bottom-original.top,SWP_NOMOVE|SWP_NOZORDER));
        CHECK(GetWindowRect(explanation,&narrow)&&block_fits(explanation));
        CHECK(narrow.bottom-narrow.top>wide.bottom-wide.top);
        CHECK(SetWindowPos(client,NULL,0,0,original.right-original.left,original.bottom-original.top,SWP_NOMOVE|SWP_NOZORDER));
        CHECK(block_fits(explanation));}
    CHECK(page(L"Hook diagnostics")==NULL);
    CHECK(GetDlgItem(page(L"Event diagnostics"),2005)&&GetDlgItem(page(L"Event diagnostics"),2018));CHECK(GetDlgItem(page(L"Event diagnostics"),2003)==NULL&&GetDlgItem(page(L"Event diagnostics"),2004)==NULL&&GetDlgItem(page(L"Event diagnostics"),2006)==NULL);CHECK(FindWindowExW(page(L"Event diagnostics"),NULL,L"SysListView32",NULL)!=NULL);
    CHECK(FindWindowExW(page(L"Help"),NULL,L"SysListView32",NULL)==NULL);
    CHECK(FindWindowExW(page(L"Help"),NULL,L"EDIT",NULL)==introduction);
    {
        const wchar_t *titles[]={L"Mapping Profile",L"Event diagnostics",L"Settings",L"Help"};
        HWND tabs=GetDlgItem(client,2000);
        CHECK((GetWindowLongW(tabs,GWL_STYLE)&TCS_OWNERDRAWFIXED)!=0);
        {LOGFONTW tab_font={0};HFONT font=(HFONT)SendMessageW(tabs,WM_GETFONT,0,0);
            CHECK(GetObjectW(font,sizeof(tab_font),&tab_font)==sizeof(tab_font));
            CHECK(tab_font.lfWeight==FW_BOLD);}
        for(unsigned i=0;i<ARRAYSIZE(titles);i++) {
            CHECK(SendMessageW(tabs,TCM_GETCURSEL,0,0)==(LRESULT)i);
            CHECK(IsWindowVisible(page(titles[i])));
            if(i+1<ARRAYSIZE(titles)){SendMessageW(tabs,WM_KEYDOWN,VK_RIGHT,0);pump(30);}
        }
    }
    puts("PASS Mapping Profile/Event diagnostics/Settings/Help order, startup checkbox, and short introduction");fflush(stdout);
    PostMessageW(open_guide,BM_CLICK,0,0);dialog=wait_guide();CHECK(dialog!=NULL);
    CHECK(GetWindow(dialog,GW_OWNER)==client&&!IsWindowEnabled(client));
    CHECK((GetWindowLongW(dialog,GWL_STYLE)&WS_THICKFRAME)!=0);
    guide=GetDlgItem(dialog,2011);CHECK(guide&&GetDlgItem(dialog,2));
    CHECK((GetWindowLongW(guide,GWL_STYLE)&ES_READONLY)!=0);
    {int length=(int)SendMessageW(guide,WM_GETTEXTLENGTH,0,0);wchar_t *content=calloc((size_t)length+1,sizeof(wchar_t));CHECK(content&&length>1000);
        SendMessageW(guide,WM_GETTEXT,(WPARAM)length+1,(LPARAM)content);int formatted=wcsstr(content,L"Typing User Guide")&&wcsstr(content,L"Get started")&&!wcsstr(content,L"# Typing User Guide")&&!wcsstr(content,L"**Ctrl+U**");free(content);CHECK(formatted);}
    {CHARFORMATW format={0};SIZE_T copied=0;format.cbSize=sizeof(format);void *remote=VirtualAllocEx(process,NULL,sizeof(format),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);CHECK(remote);
        CHARRANGE selected={0,1};int ok=WriteProcessMemory(process,remote,&selected,sizeof(selected),&copied)&&copied==sizeof(selected);
        if(ok)SendMessageW(guide,EM_EXSETSEL,0,(LPARAM)remote);
        if(ok)ok=WriteProcessMemory(process,remote,&format,sizeof(format),&copied)&&copied==sizeof(format);
        if(ok){SendMessageW(guide,EM_GETCHARFORMAT,SCF_SELECTION,(LPARAM)remote);ok=ReadProcessMemory(process,remote,&format,sizeof(format),&copied)&&copied==sizeof(format);}
        LOGFONTW label={0};HFONT label_font=(HFONT)SendMessageW(introduction,WM_GETFONT,0,0);
        CHECK(GetObjectW(label_font,sizeof(label),&label)==sizeof(label));
        LONG label_height=MulDiv(labs(label.lfHeight),1440,(int)GetDpiForWindow(introduction));
        if(!(ok&&(format.dwEffects&CFE_BOLD)&&labs(format.yHeight-label_height)<=1))printf("INFO guide formatting ok=%d effects=%lu height=%ld label=%ld\n",ok,format.dwEffects,format.yHeight,label_height);
        CHECK(ok&&(format.dwEffects&CFE_BOLD)&&labs(format.yHeight-label_height)<=1&&_wcsicmp(format.szFaceName,label.lfFaceName)==0);
        selected.cpMin=10;selected.cpMax=30;CHECK(WriteProcessMemory(process,remote,&selected,sizeof(selected),&copied));
        SendMessageW(guide,EM_EXSETSEL,0,(LPARAM)remote);
        CHECK(GetWindowRect(dialog,&outer));
        CHECK(SetWindowPos(dialog,NULL,0,0,outer.right-outer.left-80,outer.bottom-outer.top-40,SWP_NOMOVE|SWP_NOZORDER));
        SendMessageW(guide,EM_EXGETSEL,0,(LPARAM)remote);
        CHECK(ReadProcessMemory(process,remote,&selected,sizeof(selected),&copied));CHECK(selected.cpMin==10&&selected.cpMax==30);
        VirtualFreeEx(process,remote,0,MEM_RELEASE);}
    {const UINT dpis[]={96,120,144,192};
        for(unsigned i=0;i<ARRAYSIZE(dpis);i++) {
            RECT content,button;HWND close=GetDlgItem(dialog,2);
            SendMessageW(dialog,WM_DPICHANGED,MAKELONG(dpis[i],dpis[i]),0);
            CHECK(GetWindowRect(guide,&content)&&GetWindowRect(close,&button));
            CHECK(content.bottom<=button.top&&content.right>content.left&&content.bottom>content.top);
            CHECK(button.right>button.left&&button.bottom>button.top);
        }
        SendMessageW(dialog,WM_SETTINGCHANGE,0,0);
    }
    PostMessageW(dialog,WM_CLOSE,0,0);CHECK(dismissed(dialog));dialog=NULL;
    PostMessageW(open_guide,BM_CLICK,0,0);dialog=wait_guide();CHECK(dialog!=NULL);
    PostMessageW(GetDlgItem(dialog,2),BM_CLICK,0,0);CHECK(dismissed(dialog));dialog=NULL;
    PostMessageW(open_guide,BM_CLICK,0,0);dialog=wait_guide();CHECK(dialog!=NULL);
    PostMessageW(GetDlgItem(dialog,2011),WM_KEYDOWN,VK_ESCAPE,0);CHECK(dismissed(dialog));dialog=NULL;
    puts("PASS separate formatted guide, selection through resize, DPI reflow, and Close/Escape/title-bar return");fflush(stdout);
    puts("PASS UI native tabs, retained controls, and initial bounds");fflush(stdout);
    {
        const UINT dpis[]={96,120,144,192};
        for(unsigned i=0;i<ARRAYSIZE(dpis);i++) {
            RECT bounds;HFONT font;LOGFONTW logical;
            SendMessageW(client,WM_DPICHANGED,MAKELONG(dpis[i],dpis[i]),0);
            CHECK(GetWindowRect(about,&bounds));CHECK(bounds.right>bounds.left&&bounds.bottom>bounds.top);
            font=(HFONT)SendMessageW(about,WM_GETFONT,0,0);
            CHECK(GetObjectW(font,sizeof(logical),&logical)==sizeof(logical));
            CHECK(bounds.bottom-bounds.top>=labs(logical.lfHeight));
            {HWND checkbox=GetDlgItem(page(L"Settings"),2019);RECT check_bounds;
                CHECK(GetWindowRect(checkbox,&check_bounds));
                CHECK(check_bounds.right>check_bounds.left&&check_bounds.bottom-check_bounds.top>=labs(logical.lfHeight));}
            {RECT content,button;CHECK(GetWindowRect(introduction,&content));CHECK(GetWindowRect(open_guide,&button));
                CHECK(content.bottom<=bounds.top&&content.bottom<=button.top&&button.right>button.left);}
            CHECK(block_fits(introduction));
            CHECK(block_fits(FindWindowExW(page(L"Settings"),NULL,L"EDIT",NULL)));
        }
        SendMessageW(client,WM_DPICHANGED,MAKELONG(GetDpiForWindow(client),GetDpiForWindow(client)),0);
        puts("PASS simulated 100/125/150/200% DPI layout keeps native button text within bounds");
    }
    PostMessageW(client,WM_COMMAND,1002,0);CHECK(status(L"Disabled"));
    SetWindowTextW(client,L"Typing - Transition requested");
    PostMessageW(client,WM_COMMAND,1001,0);PostMessageW(client,WM_COMMAND,1002,0);
    PostMessageW(client,WM_COMMAND,1001,0);CHECK(status(L"Enabled"));
    PostMessageW(client,WM_COMMAND,1002,0);CHECK(status(L"Disabled"));
    PostMessageW(client,WM_COMMAND,1001,0);CHECK(status(L"Enabled"));
    PostMessageW(client,WM_COMMAND,1002,0);CHECK(status(L"Disabled"));
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMSUSPEND,0);CHECK(status(L"Paused"));
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMEAUTOMATIC,0);CHECK(status(L"Disabled"));
    puts("PASS UI enabled/disabled controls and disabled suspend/resume policy");fflush(stdout);
    PostMessageW(client,WM_COMMAND,1005,0);CHECK(status(L"Enabled"));
    PostMessageW(client,WM_COMMAND,1005,0);CHECK(status(L"Disabled"));
    SendMessageW(client,WM_APP+3,0,WM_LBUTTONDBLCLK);CHECK(status(L"Enabled"));
    SendMessageW(client,WM_APP+3,0,WM_LBUTTONDBLCLK);CHECK(status(L"Disabled"));
    puts("PASS tray double-click callback toggles latest requested target");
    PostMessageW(about,BM_CLICK,0,0);dialog=wait_about();CHECK(dialog!=NULL);
    CHECK(GetWindow(dialog,GW_OWNER)==client&&!IsWindowEnabled(client));
    {HWND text=about_content(dialog);wchar_t content[2048];RECT bounds={0},required={0};
        CHECK(text&&GetWindowTextW(text,content,ARRAYSIZE(content))>0);
        CHECK(wcsstr(content,L"https://github.com/aldetl/Typing")&&wcsstr(content,L"https://xdetail.org/typing"));
        CHECK(wcsstr(content,L"convenient key mapping"));
        CHECK(GetClientRect(text,&bounds));required.right=bounds.right;
        HDC dc=GetDC(text);HGDIOBJ font=SelectObject(dc,(HFONT)SendMessageW(text,WM_GETFONT,0,0));
        int height=DrawTextW(dc,content,-1,&required,DT_CALCRECT|DT_WORDBREAK|DT_NOPREFIX);
        SelectObject(dc,font);ReleaseDC(text,dc);CHECK(height>0&&height<=bounds.bottom);}
    PostMessageW(dialog,WM_CLOSE,0,0);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(GetWindowRect(client,&outer));
    SendMessageW(client,WM_ENTERSIZEMOVE,0,0);SendMessageW(client,WM_SIZING,WMSZ_BOTTOMRIGHT,(LPARAM)&outer);
    CHECK(SetWindowPos(client,NULL,0,0,outer.right-outer.left+40,outer.bottom-outer.top+30,SWP_NOMOVE|SWP_NOZORDER));
    SendMessageW(client,WM_EXITSIZEMOVE,0,0);pump(30);
    CHECK(GetClientRect(client,&rect));
    ShowWindow(client,SW_MAXIMIZE);pump(30);ShowWindow(client,SW_RESTORE);pump(30);
    {RECT restored;CHECK(GetClientRect(client,&restored));CHECK(restored.right==rect.right&&restored.bottom==rect.bottom);}
    puts("PASS UI resize and maximize/restore preserve normal size");fflush(stdout);
    PostMessageW(client,WM_CLOSE,0,0);CHECK(visible(0));CHECK(WaitForSingleObject(process,0)==WAIT_TIMEOUT);
    PostMessageW(client,WM_COMMAND,1004,0);CHECK(visible(1));
    puts("PASS UI About dialog and close-to-tray/show lifetime");fflush(stdout);
    if(inspect){puts("Inspect Typing UI; close its main window to finish this test.");fflush(stdout);
        for(unsigned i=0;i<18000&&IsWindowVisible(client)&&WaitForSingleObject(process,0)==WAIT_TIMEOUT;i++)pump(50);
    }
cleanup:
    if(dialog)PostMessageW(dialog,WM_CLOSE,0,0);
    if(client&&process&&WaitForSingleObject(process,0)==WAIT_TIMEOUT){PostMessageW(client,WM_COMMAND,1003,0);WaitForSingleObject(process,5000);}
    if(process){GetExitCodeProcess(process,&exit_code);CloseHandle(process);}
    if(started.hThread)CloseHandle(started.hThread);
    if(exit_code!=0){fwprintf(stderr,L"Client exit code: 0x%08lx\n",exit_code);failures++;}
    if(!failures&&started.hProcess&&!restart_check(argv,rect)){
        fputs("FAIL UI persisted size/restart verification\n",stderr);failures++;
    }
    if(!failures&&started.hProcess&&(!preference_boundary(argv,0)||!preference_boundary(argv,1))){
        fputs("FAIL UI invalid/unwritable preference verification\n",stderr);failures++;
    }
    if(!failures&&started.hProcess&&!logging_boundary(argv)){
        fputs("FAIL UI best-effort logging boundary\n",stderr);failures++;
    }
    puts(failures?"FAIL native UI verification":"PASS native UI verification and orderly Exit");
    return failures?1:0;
}
