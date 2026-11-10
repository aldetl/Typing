#define UNICODE
#define _UNICODE
#include <windows.h>
#include <stdio.h>
#include <wchar.h>
#include <string.h>
#include <share.h>

static DWORD target;
static HWND dialog,client;
static wchar_t details[16384];
static BOOL CALLBACK find(HWND window,LPARAM unused) {
    DWORD pid=0;wchar_t title[128],name[128];(void)unused;
    GetWindowThreadProcessId(window,&pid);if(pid!=target)return TRUE;
    GetWindowTextW(window,title,128);GetClassNameW(window,name,128);
    if(wcscmp(title,L"Typing startup error")==0)dialog=window;
    if(wcscmp(name,L"TypingWindowsClient")==0)client=window;
    return TRUE;
}
static BOOL CALLBACK read_text(HWND window,LPARAM unused) {
    wchar_t text[8192];(void)unused;GetWindowTextW(window,text,8192);
    if(wcslen(details)+wcslen(text)+2<ARRAYSIZE(details)) {wcscat_s(details,ARRAYSIZE(details),text);wcscat_s(details,ARRAYSIZE(details),L"\n");}
    return TRUE;
}
static int check(const wchar_t *host,const wchar_t *config,const wchar_t *expected,int configuration_error) {
    STARTUPINFOW startup={0};PROCESS_INFORMATION process={0};wchar_t command[16384],log[8192];
    ULONGLONG until;DWORD code=99;int ok=0;
    startup.cb=sizeof(startup);startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
    swprintf_s(log,ARRAYSIZE(log),L"%s-%lu.log",config,GetCurrentProcessId());
    swprintf_s(command,ARRAYSIZE(command),L"\"%s\" --config \"%s\" --inputsim \"missing-test-dll.dll\" --log \"%s\"",host,config,log);
    if(!CreateProcessW(host,command,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,NULL,&startup,&process))return 0;
    target=process.dwProcessId;dialog=client=NULL;details[0]=0;until=GetTickCount64()+5000;
    while(!dialog&&GetTickCount64()<until&&WaitForSingleObject(process.hProcess,0)==WAIT_TIMEOUT) {EnumWindows(find,0);Sleep(10);}
    if(dialog){
        do {
            details[0]=0;EnumChildWindows(dialog,read_text,0);
            ok=!client&&wcsstr(details,expected)&&(!configuration_error||(wcsstr(details,config)&&wcsstr(details,L"restart Typing")));
            if(!ok)Sleep(10);
        } while(!ok&&GetTickCount64()<until);
        if(ok) {
            int recorded=0;
            do {
                FILE *file=NULL;char bytes[16384];size_t size;
                file=_wfsopen(log,L"rb",_SH_DENYNO);
                if(file) {
                    size=fread(bytes,1,sizeof(bytes)-1,file);fclose(file);bytes[size]=0;
                    recorded=strstr(bytes,"INFO session: start")&&strstr(bytes,"ERROR startup:");
                }
                if(!recorded)Sleep(10);
            } while(!recorded&&GetTickCount64()<until);
            ok=recorded;
        }
        SendMessageW(dialog,WM_CLOSE,0,0);
    }
    if(WaitForSingleObject(process.hProcess,5000)!=WAIT_OBJECT_0){TerminateProcess(process.hProcess,2);ok=0;}
    if(!GetExitCodeProcess(process.hProcess,&code)||code!=1)ok=0;
    CloseHandle(process.hThread);CloseHandle(process.hProcess);
    if(!ok)fwprintf(stderr,L"FAIL startup error for %s (exit %lu, host window %p): %s\n",config,code,(void*)client,details);
    return ok;
}
int wmain(int argc,wchar_t **argv) {
    wchar_t path[4096];FILE *file=NULL;char actual[512];size_t size;
    const char *invalid="[common]\nmode_auto_exit=maybe\n[modekeys]\nCapsLock=mode\nJ=Left\n";
    if(argc!=3)return 2;
    swprintf_s(path,ARRAYSIZE(path),L"%s\\startup-missing-test.ini",argv[2]);
    if(GetFileAttributesW(path)!=INVALID_FILE_ATTRIBUTES)return 2;
    if(!check(argv[1],path,L"Cannot read configuration",1))return 1;
    swprintf_s(path,ARRAYSIZE(path),L"%s\\startup-invalid-test.ini",argv[2]);
    if(_wfopen_s(&file,path,L"wb")||!file)return 2;
    fwrite(invalid,1,strlen(invalid),file);fclose(file);
    if(!check(argv[1],path,L"on or off",1))return 1;
    if(_wfopen_s(&file,path,L"rb")||!file)return 2;
    size=fread(actual,1,sizeof(actual),file);fclose(file);
    if(size!=strlen(invalid)||memcmp(actual,invalid,size)!=0)return 1;
    swprintf_s(path,ARRAYSIZE(path),L"%s\\startup-dll-test.ini",argv[2]);
    if(_wfopen_s(&file,path,L"wb")||!file)return 2;
    fputs("[modekeys]\nCapsLock=mode\nJ=Left\n",file);fclose(file);
    if(!check(argv[1],path,L"Cannot locate InputSim DLL",0))return 1;
    puts("PASS missing/invalid configuration reports path and correction, starts no host UI, exits 1, and preserves source file");
    puts("PASS early configuration and DLL failures are recorded before host initialization");
    return 0;
}
