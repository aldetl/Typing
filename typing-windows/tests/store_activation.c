#define COBJMACROS
#define UNICODE
#define _UNICODE
#include <windows.h>
#include <shobjidl.h>
#include <commctrl.h>
#include <stdio.h>
#include <wchar.h>

static IApplicationActivationManager *activation;
static HWND client,modal;
static DWORD primary;
static HANDLE process;
static int failures;
#define CHECK(c) do { if(!(c)){fwprintf(stderr,L"FAIL Store line %d: %S (error %lu)\n",__LINE__,#c,GetLastError());failures++;goto cleanup;} } while(0)
static void pump(DWORD duration) {
    ULONGLONG until=GetTickCount64()+duration;
    do {MSG msg;while(PeekMessageW(&msg,NULL,0,0,PM_REMOVE)){TranslateMessage(&msg);DispatchMessageW(&msg);}
        if(GetTickCount64()>=until)break;MsgWaitForMultipleObjects(0,NULL,FALSE,5,QS_ALLINPUT);
    }while(1);
}
static BOOL CALLBACK find_windows(HWND window,LPARAM unused) {
    DWORD pid;wchar_t name[128],title[128];(void)unused;
    GetWindowThreadProcessId(window,&pid);if(pid!=primary)return TRUE;
    GetClassNameW(window,name,128);GetWindowTextW(window,title,128);
    if(wcscmp(name,L"TypingWindowsClient")==0)client=window;
    if(wcscmp(title,L"About TypingVE")==0)modal=window;
    return TRUE;
}
static BOOL CALLBACK find_error(HWND window,LPARAM output) {
    DWORD pid;wchar_t title[128];GetWindowThreadProcessId(window,&pid);GetWindowTextW(window,title,128);
    if(pid==primary&&wcscmp(title,L"TypingVE startup error")==0)*(HWND*)output=window;
    return TRUE;
}
static int wait_visible(int expected) {
    ULONGLONG until=GetTickCount64()+5000;
    do {EnumWindows(find_windows,0);if(client&&!!IsWindowVisible(client)==expected)return 1;pump(10);}while(GetTickCount64()<until);
    return 0;
}
static HWND page(const wchar_t *name) {
    HWND child=NULL;while((child=FindWindowExW(client,child,L"TypingPage",NULL))!=NULL){
        wchar_t title[128];GetWindowTextW(child,title,128);if(wcscmp(title,name)==0)return child;
    }return NULL;
}
static BOOL CALLBACK dump_child(HWND window,LPARAM unused) {
    wchar_t name[128],title[512];(void)unused;GetClassNameW(window,name,128);GetWindowTextW(window,title,512);
    fwprintf(stderr,L"Child %p class=%s title=%s\n",window,name,title);return TRUE;
}
static void select_page(int index) {
    HWND tabs=GetDlgItem(client,2000);int current=(int)SendMessageW(tabs,TCM_GETCURSEL,0,0);
    while(current!=index){SendMessageW(tabs,WM_KEYDOWN,current<index?VK_RIGHT:VK_LEFT,0);current+=(current<index?1:-1);}
    pump(50);
}
static int checked(HWND checkbox,int value) {
    ULONGLONG until=GetTickCount64()+5000;
    do {if(IsWindowEnabled(checkbox)&&SendMessageW(checkbox,BM_GETCHECK,0,0)==value)return 1;pump(10);}while(GetTickCount64()<until);
    {wchar_t text[2048];HWND settings=page(L"Settings"),error=FindWindowExW(settings,NULL,L"EDIT",NULL);
        error=FindWindowExW(settings,error,L"EDIT",NULL);GetWindowTextW(error,text,2048);
        fwprintf(stderr,L"Startup checkbox: HWND=%p enabled=%d checked=%lld; status=%s\n",checkbox,IsWindowEnabled(checkbox),(long long)SendMessageW(checkbox,BM_GETCHECK,0,0),text);}
    return 0;
}
static HRESULT launch(const wchar_t *aumid,const wchar_t *arguments,DWORD *pid) {
    return IApplicationActivationManager_ActivateApplication(activation,aumid,arguments,AO_NONE,pid);
}
int wmain(int argc,wchar_t **argv) {
    DWORD launched=0;HWND checkbox;wchar_t title[256];HRESULT hr;
    if(argc<2||argc>4)return 2;
    hr=CoInitializeEx(NULL,COINIT_APARTMENTTHREADED);if(FAILED(hr))return 2;
    hr=CoCreateInstance(&CLSID_ApplicationActivationManager,NULL,CLSCTX_LOCAL_SERVER,&IID_IApplicationActivationManager,(void**)&activation);
    CHECK(SUCCEEDED(hr));
    if(wcscmp(argv[1],L"--activate")==0&&argc>=3){
        hr=launch(argv[2],argc==4?argv[3]:L"",&launched);CHECK(SUCCEEDED(hr));
        wprintf(L"%lu\n",launched);goto cleanup;
    }
    if(wcscmp(argv[1],L"--expect-error")==0&&argc==4){
        HWND error=NULL;DWORD code=99;
        CHECK(SUCCEEDED(launch(argv[2],argv[3],&primary)));
        process=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,primary);CHECK(process);
        {ULONGLONG until=GetTickCount64()+5000;do{EnumWindows(find_error,(LPARAM)&error);if(error&&IsWindowVisible(error))break;pump(10);}while(GetTickCount64()<until);}
        CHECK(error&&IsWindowVisible(error));PostMessageW(error,WM_CLOSE,0,0);
        CHECK(WaitForSingleObject(process,5000)==WAIT_OBJECT_0);CHECK(GetExitCodeProcess(process,&code)&&code==1);
        CloseHandle(process);process=NULL;puts("PASS packaged fatal-error dialog and exit status");goto cleanup;
    }
    CHECK(argc==2);
    CHECK(SUCCEEDED(launch(argv[1],L"--startup",&primary)));
    process=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,primary);CHECK(process);
    CHECK(wait_visible(0));
    {ULONGLONG until=GetTickCount64()+5000;do{GetWindowTextW(client,title,256);if(wcsstr(title,L"Enabled"))break;pump(10);}while(GetTickCount64()<until);CHECK(wcsstr(title,L"Enabled"));}
    puts("PASS packaged startup intent stays hidden");
    CHECK(SUCCEEDED(launch(argv[1],L"",&launched)));CHECK(wait_visible(1));
    CHECK(WaitForSingleObject(process,0)==WAIT_TIMEOUT);
    select_page(2);checkbox=GetDlgItem(page(L"Settings"),2019);
    if(!checkbox){DWORD pid=0;GetWindowThreadProcessId(client,&pid);fwprintf(stderr,L"Primary=%lu window=%p owner=%lu state=%lu\n",primary,client,pid,WaitForSingleObject(process,0));EnumChildWindows(client,dump_child,0);}
    CHECK(checkbox&&checked(checkbox,BST_UNCHECKED));
    PostMessageW(checkbox,BM_CLICK,0,0);CHECK(checked(checkbox,BST_CHECKED));
    PostMessageW(checkbox,BM_CLICK,0,0);CHECK(checked(checkbox,BST_UNCHECKED));
    puts("PASS packaged startup task reads and changes asynchronously");
    PostMessageW(client,WM_COMMAND,1002,0);pump(100);
    PostMessageW(client,WM_CLOSE,0,0);CHECK(wait_visible(0));
    CHECK(SUCCEEDED(launch(argv[1],L"--startup",&launched)));pump(300);CHECK(!IsWindowVisible(client));
    CHECK(SUCCEEDED(launch(argv[1],L"",&launched)));CHECK(wait_visible(1));
    CHECK(SendMessageW(GetDlgItem(client,2000),TCM_GETCURSEL,0,0)==2);
    GetWindowTextW(client,title,256);CHECK(wcsstr(title,L"Disabled"));
    puts("PASS repeated Show preserves page and enablement; startup does not raise it");
    select_page(3);PostMessageW(GetDlgItem(page(L"Help"),2007),BM_CLICK,0,0);
    {ULONGLONG until=GetTickCount64()+5000;do{EnumWindows(find_windows,0);if(modal)break;pump(10);}while(GetTickCount64()<until);}
    CHECK(modal&&!IsWindowEnabled(client));
    CHECK(SUCCEEDED(launch(argv[1],L"",&launched)));pump(200);
    CHECK(IsWindow(modal)&&IsWindowVisible(modal)&&!IsWindowEnabled(client));
    PostMessageW(modal,WM_CLOSE,0,0);pump(50);modal=NULL;
    puts("PASS repeated Show preserves an owned modal interaction");
cleanup:
    if(modal)PostMessageW(modal,WM_CLOSE,0,0);
    if(client&&process&&WaitForSingleObject(process,0)==WAIT_TIMEOUT){PostMessageW(client,WM_COMMAND,1003,0);if(WaitForSingleObject(process,5000)!=WAIT_OBJECT_0)failures++;}
    if(process){DWORD code=99;GetExitCodeProcess(process,&code);if(code!=0)failures++;CloseHandle(process);}
    if(activation)IApplicationActivationManager_Release(activation);
    CoUninitialize();return failures?1:0;
}
