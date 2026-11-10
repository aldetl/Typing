#define UNICODE
#define _UNICODE
#include <windows.h>
#include <commctrl.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <stddef.h>

/* An isolated actual-client test. All files live in the caller's test directory. */
static DWORD pid;
static HWND client;
static HANDLE process;
static int failed;
static HWND controller;
#define CHECK(c) do { if (!(c)) { fwprintf(stderr,L"FAIL profile UI line %d: %S (error %lu)\n",__LINE__,#c,GetLastError()); failed=1; goto cleanup; } } while(0)
/* File APIs accept '/', but the native file picker rejects it in filenames.
   Resolve caller-relative arguments before the child changes working directory. */
static int absolute_windows_path(const wchar_t *source,wchar_t *destination,DWORD capacity) {
    if(!source[0]){SetLastError(ERROR_INVALID_NAME);return 0;}
    DWORD length=GetFullPathNameW(source,capacity,destination,NULL);
    if(!length)return 0;
    if(length>=capacity){SetLastError(ERROR_FILENAME_EXCED_RANGE);return 0;}
    for(DWORD i=0;i<length;i++)if(destination[i]==L'/')destination[i]=L'\\';
    return 1;
}
static int check_paths(void) {
    const struct { const wchar_t *source,*expected; } cases[]={
        {L"C:\\Typing Test\\profiles\\example.typing.ini",L"C:\\Typing Test\\profiles\\example.typing.ini"},
        {L"C:/Typing Test/profiles/example.typing.ini",L"C:\\Typing Test\\profiles\\example.typing.ini"},
        {L"C:\\Typing Test/profiles\\example.typing.ini",L"C:\\Typing Test\\profiles\\example.typing.ini"},
        {L"C:/Typing Test/old/../profiles/./example.typing.ini",L"C:\\Typing Test\\profiles\\example.typing.ini"},
        {L"//typing-test/share/profiles/example.typing.ini",L"\\\\typing-test\\share\\profiles\\example.typing.ini"},
    };
    wchar_t result[4096],directory[4096],expected[4096];
    for(unsigned i=0;i<ARRAYSIZE(cases);i++) {
        if(!absolute_windows_path(cases[i].source,result,ARRAYSIZE(result))||wcscmp(result,cases[i].expected)!=0) {
            fwprintf(stderr,L"FAIL profile test path normalization case %u\n",i);return 1;
        }
    }
    DWORD length=GetCurrentDirectoryW(ARRAYSIZE(directory),directory);
    if(!length||length>=ARRAYSIZE(directory))return 1;
    swprintf_s(expected,ARRAYSIZE(expected),L"%s%starget\\Profile Test\\example.typing.ini",directory,
        directory[length-1]==L'\\'?L"":L"\\");
    if(!absolute_windows_path(L"./target/Profile Test/old/../example.typing.ini",result,ARRAYSIZE(result))||wcscmp(result,expected)!=0) {
        fputs("FAIL caller-relative profile test path normalization\n",stderr);return 1;
    }
    if(absolute_windows_path(L"C:/Typing Test",result,4)||GetLastError()!=ERROR_FILENAME_EXCED_RANGE||
        absolute_windows_path(L"",result,ARRAYSIZE(result))||GetLastError()!=ERROR_INVALID_NAME) {
        fputs("FAIL invalid or oversized profile test path rejection\n",stderr);return 1;
    }
    puts("PASS profile test paths: backslashes, forward/mixed slashes, spaces, relative paths, UNC, and rejection boundaries");
    return 0;
}
static void pump(DWORD milliseconds) {
    ULONGLONG until=GetTickCount64()+milliseconds;
    do {MSG message;while(PeekMessageW(&message,NULL,0,0,PM_REMOVE)){TranslateMessage(&message);DispatchMessageW(&message);}if(GetTickCount64()>=until)break;MsgWaitForMultipleObjects(0,NULL,FALSE,10,QS_ALLINPUT);}while(1);
}
struct search { const wchar_t *class_name; HWND result; };
static BOOL CALLBACK find_window(HWND window, LPARAM value) {
    struct search *search=(struct search*)value; DWORD owner=0; wchar_t name[128];
    GetWindowThreadProcessId(window,&owner); GetClassNameW(window,name,128);
    if(owner==pid&&wcscmp(name,search->class_name)==0)search->result=window;
    return TRUE;
}
static HWND wait_window(const wchar_t *class_name) {
    ULONGLONG until=GetTickCount64()+5000; struct search search={class_name,NULL};
    do { EnumWindows(find_window,(LPARAM)&search); if(search.result&&(wcscmp(class_name,L"TypingWindowsClient")==0||IsWindowVisible(search.result)))return search.result; pump(10); } while(GetTickCount64()<until);
    return NULL;
}
static HWND page(void) { HWND next=NULL; while((next=FindWindowExW(client,next,L"TypingPage",NULL))!=NULL) {
    wchar_t title[128];GetWindowTextW(next,title,128);if(wcscmp(title,L"Mapping Profile")==0)return next;
} return NULL; }
static int has_text(HWND window,const wchar_t *expected) { wchar_t buffer[8192];SendMessageW(window,WM_GETTEXT,ARRAYSIZE(buffer),(LPARAM)buffer);return wcsstr(buffer,expected)!=NULL; }
static void set_value(HWND window,const wchar_t *value) {SendMessageW(window,WM_SETTEXT,0,(LPARAM)value);}
struct text_search { const wchar_t *text; int found; };
static BOOL CALLBACK child_text(HWND window,LPARAM value) {struct text_search *search=(struct text_search*)value;if(has_text(window,search->text))search->found=1;return TRUE;}
static int wait_text(HWND window,const wchar_t *expected) {
    ULONGLONG until=GetTickCount64()+5000;
    do {struct text_search search={expected,has_text(window,expected)};EnumChildWindows(window,child_text,(LPARAM)&search);if(search.found)return 1;if(WaitForSingleObject(process,0)!=WAIT_TIMEOUT)return 0;pump(10);}while(GetTickCount64()<until);
    return 0;
}
static int wait_letter(HWND window) {
    ULONGLONG until=GetTickCount64()+5000;wchar_t buffer[128]={0};
    do {SendMessageW(window,WM_GETTEXT,ARRAYSIZE(buffer),(LPARAM)buffer);if(_wcsicmp(buffer,L"j")==0)return 1;pump(10);}while(GetTickCount64()<until);
    fwprintf(stderr,L"INFO editor received '%s' while testing a key bound as an always-active hotkey\n",buffer);return 0;
}
static int dismissed(HWND window) { ULONGLONG until=GetTickCount64()+5000;while(IsWindow(window)&&GetTickCount64()<until)pump(10);if(IsWindow(window)){wchar_t title[256],cls[128];GetWindowTextW(window,title,256);GetClassNameW(window,cls,128);fwprintf(stderr,L"INFO undismissed window %p %s %s\n",(void*)window,cls,title);}return !IsWindow(window); }
static int wait_enabled(HWND window) { ULONGLONG until=GetTickCount64()+5000;while(!IsWindowEnabled(window)&&GetTickCount64()<until)pump(10);return IsWindowEnabled(window); }
static int write_file(const wchar_t *path,const char *source) {FILE *file=NULL;if(_wfopen_s(&file,path,L"wb")||!file)return 0;size_t length=strlen(source);int ok=fwrite(source,1,length,file)==length;return fclose(file)==0&&ok;}
static int file_contains(const wchar_t *path,const char *expected) {FILE *file=NULL;char buffer[32768];size_t length;if(_wfopen_s(&file,path,L"rb")||!file)return 0;length=fread(buffer,1,sizeof(buffer)-1,file);buffer[length]=0;fclose(file);return strstr(buffer,expected)!=NULL;}
static int wait_file(const wchar_t *path,const char *expected) {ULONGLONG until=GetTickCount64()+5000;do{if(file_contains(path,expected))return 1;pump(10);}while(GetTickCount64()<until);return 0;}
static BOOL CALLBACK find_filename(HWND window,LPARAM value) {
    wchar_t name[128];int id=GetDlgCtrlID(window);GetClassNameW(window,name,128);
    if(wcscmp(name,L"Edit")==0&&(id==1148||id==1152||id==1001))*(HWND*)value=window;
    return TRUE;
}
static int choose_file(HWND dialog,const wchar_t *path) {
    HWND edit=NULL;EnumChildWindows(dialog,find_filename,(LPARAM)&edit);
    if(!edit)return 0;
    set_value(edit,path);PostMessageW(dialog,WM_COMMAND,IDOK,0);return 1;
}
static HWND add_dialog(void) {PostMessageW(client,WM_COMMAND,3007,0);return wait_window(L"TypingBindingEditor");}
static HWND first_edit(HWND dialog) {return FindWindowExW(dialog,NULL,L"EDIT",NULL);}
static HWND read_only_block(HWND parent) {
    HWND block=NULL;while((block=FindWindowExW(parent,block,L"EDIT",NULL))!=NULL)
        if(GetWindowLongW(block,GWL_STYLE)&ES_READONLY)return block;
    return NULL;
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
static int compact_control(HWND window) {
    RECT bounds={0};LOGFONTW font={0};HFONT handle=(HFONT)SendMessageW(window,WM_GETFONT,0,0);
    if(!GetWindowRect(window,&bounds)||GetObjectW(handle,sizeof(font),&font)!=sizeof(font)){printf("INFO compact font unavailable: %p\n",(void*)handle);return 0;}
    LONG height=bounds.bottom-bounds.top;return height>=labs(font.lfHeight)&&height<=2*labs(font.lfHeight)+4;
}
static int wait_compact(HWND window) {ULONGLONG until=GetTickCount64()+5000;do{if(compact_control(window))return 1;pump(10);}while(GetTickCount64()<until);return 0;}
static void command(HWND window,UINT id) {
    if(window==client&&id>=3000&&id<=3013)wait_enabled(GetDlgItem(page(),id));
    if(window!=client&&id==IDOK) {HWND button=FindWindowExW(window,NULL,L"Button",NULL);if(button){SendMessageW(button,BM_CLICK,0,0);return;}}
    PostMessageW(window,WM_COMMAND,id,0);
}
static int select_mapping(HWND list,int index) {
    LVITEMW item={0};SIZE_T written=0;item.state=LVIS_SELECTED|LVIS_FOCUSED;item.stateMask=item.state;
    void *remote=VirtualAllocEx(process,NULL,sizeof(item),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!remote)return 0;
    int ok=WriteProcessMemory(process,remote,&item,sizeof(item),&written)&&written==sizeof(item);
    if(ok)ok=(int)SendMessageW(list,LVM_SETITEMSTATE,index,(LPARAM)remote);
    VirtualFreeEx(process,remote,0,MEM_RELEASE);return ok;
}
static int sort_mappings(HWND list,int column) {
    HWND header=(HWND)SendMessageW(list,LVM_GETHEADER,0,0);RECT bounds={0};SIZE_T count=0;
    void *remote=VirtualAllocEx(process,NULL,sizeof(bounds),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!remote)return 0;
    int ok=(int)SendMessageW(header,HDM_GETITEMRECT,column,(LPARAM)remote)&&ReadProcessMemory(process,remote,&bounds,sizeof(bounds),&count)&&count==sizeof(bounds);
    if(ok){LPARAM point=MAKELPARAM((bounds.left+bounds.right)/2,(bounds.top+bounds.bottom)/2);SendMessageW(header,WM_LBUTTONDOWN,MK_LBUTTON,point);SendMessageW(header,WM_LBUTTONUP,0,point);}
    VirtualFreeEx(process,remote,0,MEM_RELEASE);return ok;
}
static int mapping_cell(HWND list,int index,int column,const wchar_t *expected) {
    struct cell_value {LVITEMW item;wchar_t text[256];} value={0};SIZE_T transferred=0;
    void *remote=VirtualAllocEx(process,NULL,sizeof(value),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    if(!remote)return 0;
    value.item.iSubItem=column;value.item.cchTextMax=ARRAYSIZE(value.text);value.item.pszText=(wchar_t*)((BYTE*)remote+offsetof(struct cell_value,text));
    int ok=WriteProcessMemory(process,remote,&value,sizeof(value),&transferred)&&transferred==sizeof(value);
    if(ok){SendMessageW(list,LVM_GETITEMTEXTW,index,(LPARAM)remote);ok=ReadProcessMemory(process,remote,&value,sizeof(value),&transferred)&&transferred==sizeof(value);if(ok&&wcscmp(value.text,expected)!=0){fwprintf(stderr,L"INFO mapping row %d column %d expected '%s', actual '%s'\n",index,column,expected,value.text);ok=0;}}
    VirtualFreeEx(process,remote,0,MEM_RELEASE);return ok;
}
static int wait_check(HWND list,int index,int state) {
    ULONGLONG until=GetTickCount64()+5000;
    do{if((SendMessageW(list,LVM_GETITEMSTATE,index,LVIS_STATEIMAGEMASK)>>12)==state)return 1;pump(10);}while(GetTickCount64()<until);
    return 0;
}
static int mouse_toggle_first(HWND list) {
    RECT bounds={0};SIZE_T count=0;bounds.left=LVIR_BOUNDS;
    void *remote=VirtualAllocEx(process,NULL,sizeof(bounds),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote)return 0;
    int ok=WriteProcessMemory(process,remote,&bounds,sizeof(bounds),&count)&&count==sizeof(bounds);
    if(ok)ok=(int)SendMessageW(list,LVM_GETITEMRECT,0,(LPARAM)remote)&&ReadProcessMemory(process,remote,&bounds,sizeof(bounds),&count)&&count==sizeof(bounds);
    VirtualFreeEx(process,remote,0,MEM_RELEASE);
    if(ok){LPARAM point=MAKELPARAM(bounds.left+8,(bounds.top+bounds.bottom)/2);SendMessageW(list,WM_LBUTTONDOWN,MK_LBUTTON,point);SendMessageW(list,WM_LBUTTONUP,0,point);}
    return ok;
}
static int probe_j(int mapped) {
    HWND edit=CreateWindowW(L"EDIT",L"ab",WS_CHILD|WS_VISIBLE|ES_AUTOHSCROLL,10,10,300,30,controller,NULL,GetModuleHandleW(NULL),NULL);
    if(!edit)return 0;
    SendMessageW(client,WM_CLOSE,0,0);SetForegroundWindow(controller);SetFocus(edit);pump(100);
    int ok=GetForegroundWindow()==controller;
    if(ok){
        SendMessageW(edit,EM_SETSEL,1,1);INPUT keys[2]={0};keys[0].type=INPUT_KEYBOARD;keys[0].ki.wVk='J';keys[1]=keys[0];keys[1].ki.dwFlags=KEYEVENTF_KEYUP;
        ok=SendInput(2,keys,sizeof(INPUT))==2;pump(200);
        wchar_t value[32];GetWindowTextW(edit,value,ARRAYSIZE(value));
        ok=ok&&_wcsicmp(value,mapped?L"ab":L"ajb")==0&&(!mapped||SendMessageW(edit,EM_GETSEL,0,0)==0);
        if(!ok)fwprintf(stderr,L"INFO probe J: mapped=%d text='%s' selection=%lld\n",mapped,value,(long long)SendMessageW(edit,EM_GETSEL,0,0));
    }
    else fputs("INFO probe J: controller could not acquire foreground focus; no input sent\n",stderr);
    DestroyWindow(edit);command(client,1004);pump(100);return ok;
}
static int chord_input(HWND dialog) {
    INPUT events[6]={0};const WORD keys[]={VK_LCONTROL,VK_LSHIFT,VK_F11,VK_F11,VK_LSHIFT,VK_LCONTROL};
    for(int i=0;i<6;i++){events[i].type=INPUT_KEYBOARD;events[i].ki.wVk=keys[i];if(i>=3)events[i].ki.dwFlags=KEYEVENTF_KEYUP;}
    SetForegroundWindow(dialog);pump(100);if(GetForegroundWindow()!=dialog)return 0;
    return SendInput(6,events,sizeof(INPUT))==6;
}
static int missed_release_recovery(HWND dialog,const wchar_t *target) {
    INPUT key={0};key.type=INPUT_KEYBOARD;key.ki.wVk='J';
    if(GetForegroundWindow()!=dialog||SendInput(1,&key,sizeof(key))!=1)return 0;
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMSUSPEND,0);if(!wait_text(client,L"Paused"))return 0;
    key.ki.dwFlags=KEYEVENTF_KEYUP;
#ifdef _WIN64
    key.ki.dwExtraInfo=(ULONG_PTR)(0x5459504900000000ULL|pid);
#else
    key.ki.dwExtraInfo=(ULONG_PTR)pid;
#endif
    /* A tagged release updates native state without entering the recorder,
       modeling a physical release missed during desktop loss. */
    if(SendInput(1,&key,sizeof(key))!=1)return 0;
    PostMessageW(client,WM_POWERBROADCAST,PBT_APMRESUMEAUTOMATIC,0);if(!wait_text(client,target))return 0;
    INPUT typed[2]={0};typed[0].type=INPUT_KEYBOARD;typed[0].ki.wVk='J';typed[1]=typed[0];typed[1].ki.dwFlags=KEYEVENTF_KEYUP;
    return SendInput(2,typed,sizeof(INPUT))==2&&wait_letter(first_edit(dialog));
}

static int named_action(UINT id,const wchar_t *name) {
    command(client,id);HWND dialog=wait_window(L"TypingProfileNameDialog");if(!dialog)return 0;
    if(!compact_control(GetDlgItem(dialog,3200)))return 0;
    set_value(GetDlgItem(dialog,3200),name);command(dialog,3201);return dismissed(dialog);
}
static int select_profile(const wchar_t *name) {
    HWND combo=GetDlgItem(page(),3001);ULONGLONG until=GetTickCount64()+5000;LRESULT index;
    do {index=SendMessageW(combo,CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)name);if(index!=CB_ERR)break;pump(10);}while(GetTickCount64()<until);
    if(index==CB_ERR)return 0;
    if(!wait_enabled(combo))return 0;
    SendMessageW(combo,CB_SETCURSEL,(WPARAM)index,0);
    return PostMessageW(client,WM_COMMAND,MAKEWPARAM(3001,CBN_SELCHANGE),(LPARAM)combo)!=0;
}
static int activate_profile(const wchar_t *name) {
    wchar_t expected[512];swprintf_s(expected,ARRAYSIZE(expected),L"Editing: %s",name);
    if(!wait_text(page(),expected)||!wait_enabled(GetDlgItem(page(),3014)))return 0;
    SendMessageW(client,WM_COMMAND,3014,0);
    if(!wait_enabled(GetDlgItem(page(),3014)))return 0;
    swprintf_s(expected,ARRAYSIZE(expected),L"Active: %s",name);
    return wait_text(page(),expected);
}
static int has_profile(const wchar_t *name) {
    return SendMessageW(GetDlgItem(page(),3001),CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)name)!=CB_ERR;
}
static int choose_delete(const wchar_t *name) {
    command(client,3005);HWND dialog=wait_window(L"TypingProfileNameDialog");if(!dialog)return 0;
    HWND combo=GetDlgItem(dialog,3200);LRESULT index=SendMessageW(combo,CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)name);
    if(index==CB_ERR){command(dialog,3202);return 0;}
    SendMessageW(combo,CB_SETCURSEL,(WPARAM)index,0);command(dialog,3201);return dismissed(dialog);
}
int wmain(int argc,wchar_t **argv) {
    wchar_t config[4096],profile[4096],other[4096],external[4096],clone[4096],empty[4096],imported[4096],legacy[4096],legacy_copy[4096],invalid[4096],root[4096],nested[4096],host[4096],log[4096],command_line[16384],backup[4096];
    wchar_t application[4096],inputsim[4096],test_directory[4096];
    HWND binding_page=NULL,dialog=NULL,input=NULL,scope=NULL,action=NULL,list=NULL,file=NULL;
    PROCESS_INFORMATION child={0};STARTUPINFOW startup={0};DWORD exit_code=99;
    DWORD original_attributes=INVALID_FILE_ATTRIBUTES;
    if(argc==2&&wcscmp(argv[1],L"--check-paths")==0)return check_paths();
    SetProcessDPIAware();
    if(argc==3&&wcscmp(argv[1],L"--quit")==0) {
        pid=wcstoul(argv[2],NULL,10);process=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);if(!process)return 2;
        client=wait_window(L"TypingWindowsClient");if(client)command(client,1003);
        int ok=WaitForSingleObject(process,5000)==WAIT_OBJECT_0;CloseHandle(process);return ok?0:1;
    }
    if(argc!=4&&!(argc==5&&(wcscmp(argv[4],L"--unusable-log")==0||wcscmp(argv[4],L"--inspect")==0))){fwprintf(stderr,L"Usage: profiles_ui HOST INPUTSIM TEST_DIRECTORY [--unusable-log|--inspect]\n       profiles_ui --check-paths\n");return 2;}
    if(!absolute_windows_path(argv[1],application,ARRAYSIZE(application))||
       !absolute_windows_path(argv[2],inputsim,ARRAYSIZE(inputsim))||
       !absolute_windows_path(argv[3],test_directory,ARRAYSIZE(test_directory))) {
        fwprintf(stderr,L"Cannot resolve profile test paths (error %lu)\n",GetLastError());return 2;
    }
    {WNDCLASSW cls={0};cls.lpfnWndProc=DefWindowProcW;cls.hInstance=GetModuleHandleW(NULL);cls.lpszClassName=L"TypingProfileTestController";RegisterClassW(&cls);
        controller=CreateWindowW(cls.lpszClassName,L"Typing profile test controller",WS_OVERLAPPEDWINDOW|WS_VISIBLE,100,100,600,160,NULL,NULL,cls.hInstance,NULL);
        CHECK(controller);SetForegroundWindow(controller);puts("INFO activate Typing profile test controller to begin the isolated native test");fflush(stdout);
        ULONGLONG until=GetTickCount64()+60000;while(GetForegroundWindow()!=controller&&GetTickCount64()<until)pump(20);CHECK(GetForegroundWindow()==controller);
    }
    CreateDirectoryW(test_directory,NULL);
    swprintf_s(root,4096,L"%s\\profiles",test_directory);CHECK(CreateDirectoryW(root,NULL));
    swprintf_s(nested,4096,L"%s\\nested",root);CHECK(CreateDirectoryW(nested,NULL));
    swprintf_s(config,4096,L"%s\\configuration.ini",test_directory);swprintf_s(profile,4096,L"%s\\layout.typing.ini",root);
    swprintf_s(other,4096,L"%s\\other.typing.ini",root);swprintf_s(external,4096,L"%s\\external.typing.ini",test_directory);
    swprintf_s(clone,4096,L"%s\\profile-Clone.typing.ini",root);swprintf_s(empty,4096,L"%s\\profile-Empty.typing.ini",root);
    swprintf_s(imported,4096,L"%s\\profile-Imported.typing.ini",root);swprintf_s(legacy_copy,4096,L"%s\\profile-Legacy copy.typing.ini",root);
    swprintf_s(legacy,4096,L"%s\\legacy.ini",test_directory);swprintf_s(invalid,4096,L"%s\\invalid.typing.ini",test_directory);
    swprintf_s(host,4096,L"%s\\Typing.exe",test_directory);CHECK(CopyFileW(application,host,TRUE));
    swprintf_s(log,4096,L"%s\\host.log",test_directory);swprintf_s(backup,4096,L"%s.bak",profile);
    CHECK(write_file(config,"; application settings stay independent\r\n[common]\r\nhotkey=Control+Shift+F11\r\nmode_auto_exit=off\r\n[profiles]\r\nactive=profiles/layout.typing.ini\r\n"));
    CHECK(write_file(profile,"[profile]\r\nversion=1\r\nname=First\r\n[modekeys]\r\n; keep this movement comment\r\nCapsLock=mode\r\nJ=Left\r\n[hotkeys]\r\nJ=Left\r\n"));
    CHECK(write_file(other,"[profile]\r\nversion=1\r\nname=Other\r\n[modekeys]\r\n[hotkeys]\r\n"));
    {wchar_t path[4096];swprintf_s(path,4096,L"%s\\outside.typing.ini",test_directory);CHECK(write_file(path,"[profile]\r\nversion=1\r\nname=Outside\r\n[modekeys]\r\n[hotkeys]\r\n"));}
    {wchar_t path[4096];swprintf_s(path,4096,L"%s\\hidden.typing.ini",nested);CHECK(write_file(path,"[profile]\nversion=1\nname=Nested\n[modekeys]\n[hotkeys]\n"));}
    CHECK(write_file(legacy,"[modekeys]\r\nCapsLock=mode\r\nJ=Left\r\n[hotkeys]\r\nJ=Left\r\n"));
    CHECK(write_file(invalid,"[profile]\r\nversion=99\r\nname=Invalid\r\n[modekeys]\r\n[hotkeys]\r\n"));
    swprintf_s(command_line,16384,L"\"%s\" --config \"%s\" --inputsim \"%s\" --log \"%s\" --state \"%s.state\"",host,config,inputsim,log,log);
    if(argc==5&&wcscmp(argv[4],L"--unusable-log")==0&&!CreateDirectoryW(log,NULL)&&GetLastError()!=ERROR_ALREADY_EXISTS)return 2;
    startup.cb=sizeof(startup);startup.dwFlags=STARTF_USESHOWWINDOW;startup.wShowWindow=SW_HIDE;
    CHECK(CreateProcessW(host,command_line,NULL,NULL,FALSE,CREATE_NO_WINDOW,NULL,test_directory,&startup,&child));
    process=child.hProcess;pid=child.dwProcessId;client=wait_window(L"TypingWindowsClient");CHECK(client);CHECK(wait_text(client,L"Enabled"));AllowSetForegroundWindow(pid);
    command(client,1004);binding_page=page();CHECK(binding_page);list=FindWindowExW(binding_page,NULL,L"SysListView32",NULL);CHECK(list);
    CHECK(SendMessageW(GetDlgItem(client,2000),TCM_GETCURSEL,0,0)==0&&IsWindowVisible(binding_page));
    CHECK(wait_text(binding_page,L"Active: First"));CHECK(SendMessageW(GetDlgItem(binding_page,3001),CB_GETCOUNT,0,0)==2);
    CHECK(has_profile(L"First")&&has_profile(L"Other")&&!has_profile(L"Outside")&&!has_profile(L"Nested"));
    CHECK(GetDlgItem(binding_page,3012)==NULL&&GetDlgItem(binding_page,3010)==NULL&&GetDlgItem(binding_page,3011)==NULL);
    CHECK(read_only_block(binding_page)&&block_fits(read_only_block(binding_page)));
    {HWND combo=GetDlgItem(binding_page,3001);SendMessageW(combo,CB_SHOWDROPDOWN,TRUE,0);pump(30);CHECK(SendMessageW(combo,CB_GETDROPPEDSTATE,0,0));SendMessageW(combo,CB_SHOWDROPDOWN,FALSE,0);}
    command(client,3005);dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);
    CHECK(SendMessageW(GetDlgItem(dialog,3200),CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)L"First")==CB_ERR);
    CHECK(SendMessageW(GetDlgItem(dialog,3200),CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)L"Other")!=CB_ERR);
    command(dialog,3202);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(wait_compact(GetDlgItem(binding_page,3002)));CHECK(wait_text(binding_page,L"Key mapping:"));
    {RECT table,add;CHECK(GetWindowRect(list,&table)&&GetWindowRect(GetDlgItem(binding_page,3007),&add));CHECK(add.bottom<=table.top);}
    puts("PASS name-only nonrecursive profile dropdown, seven actions, active deletion protection, and compact mapping layout");fflush(stdout);
    if(argc==5&&wcscmp(argv[4],L"--inspect")==0){puts("INFO inspect Mapping Profile; close the client window to finish");fflush(stdout);ULONGLONG until=GetTickCount64()+180000;while(IsWindowVisible(client)&&GetTickCount64()<until&&WaitForSingleObject(process,0)==WAIT_TIMEOUT)pump(20);goto cleanup;}
    {const wchar_t *long_name=L"A long profile name with enough words to wrap the complete editing status over several lines in a narrow window without losing whether the saved mappings are active";
        HWND status_block=read_only_block(binding_page);RECT original,grown,shrunk,add;
        CHECK(GetWindowRect(client,&original));CHECK(named_action(3013,long_name));
        CHECK(SetWindowPos(client,NULL,0,0,MulDiv(420,(int)GetDpiForWindow(client),96),original.bottom-original.top,SWP_NOMOVE|SWP_NOZORDER));pump(100);
        CHECK(wait_text(binding_page,long_name)&&block_fits(status_block));
        CHECK(SendMessageW(status_block,EM_GETLINECOUNT,0,0)>2);
        CHECK(GetWindowRect(status_block,&grown)&&GetWindowRect(GetDlgItem(binding_page,3007),&add)&&grown.bottom<=add.top);
        CHECK(named_action(3013,L"First"));pump(100);
        CHECK(GetWindowRect(status_block,&shrunk)&&shrunk.bottom-shrunk.top<grown.bottom-grown.top&&block_fits(status_block));
        CHECK(SetWindowPos(client,NULL,0,0,original.right-original.left,original.bottom-original.top,SWP_NOMOVE|SWP_NOZORDER));
    }
    {RECT original,wide,narrow,status_bounds,confirm_bounds;HWND status_block;
        dialog=add_dialog();CHECK(dialog);status_block=read_only_block(dialog);CHECK(status_block&&block_fits(status_block));
        CHECK(GetWindowRect(dialog,&original)&&GetWindowRect(status_block,&wide));
        CHECK(SetWindowPos(dialog,NULL,0,0,MulDiv(340,(int)GetDpiForWindow(dialog),96),original.bottom-original.top,SWP_NOMOVE|SWP_NOZORDER));pump(100);
        CHECK(GetWindowRect(status_block,&narrow)&&narrow.bottom-narrow.top>wide.bottom-wide.top&&block_fits(status_block));
        set_value(status_block,L"A long changing validation message with several words and an unbroken name AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA must remain readable with all its lines while Confirm and Cancel follow the expanded message.");pump(100);
        CHECK(block_fits(status_block)&&GetWindowRect(status_block,&status_bounds)&&GetWindowRect(GetDlgItem(dialog,3104),&confirm_bounds));
        CHECK(labs((confirm_bounds.top-status_bounds.bottom)*96-10*(LONG)GetDpiForWindow(dialog))<96);
        command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;
    }
    puts("PASS common text blocks: long-name/status reflow, narrow dialog guidance, changing messages, unbroken words, and shared spacing");fflush(stdout);
    CHECK(wait_check(list,0,2)&&wait_check(list,1,2)&&wait_check(list,2,2));
    CHECK(sort_mappings(list,2));CHECK(sort_mappings(list,2));
    CHECK(mapping_cell(list,0,1,L"Command mode")&&mapping_cell(list,0,2,L"J"));
    CHECK(select_mapping(list,0)&&mouse_toggle_first(list));CHECK(wait_check(list,0,1));
    CHECK(wait_text(binding_page,L"Unsaved changes")&&!file_contains(profile,"!J=Left"));
    CHECK(SendMessageW(list,LVM_GETNEXTITEM,(WPARAM)-1,LVNI_SELECTED)==0);
    CHECK(select_mapping(list,0));command(client,3008);dialog=wait_window(L"TypingBindingEditor");CHECK(dialog);
    CHECK(SendMessageW(GetDlgItem(dialog,3107),BM_GETCHECK,0,0)==BST_UNCHECKED);
    SendMessageW(GetDlgItem(dialog,3107),BM_CLICK,0,0);command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;CHECK(wait_check(list,0,1));
    CHECK(select_mapping(list,0));command(client,3008);dialog=wait_window(L"TypingBindingEditor");CHECK(dialog);command(dialog,3104);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(wait_check(list,0,1));
    /* Keyboard Space changes the selected source entry after sorting. */
    CHECK(select_mapping(list,0));SendMessageW(list,WM_KEYDOWN,VK_SPACE,0);SendMessageW(list,WM_KEYUP,VK_SPACE,0);CHECK(wait_check(list,0,2));
    /* Disable both J mappings through separate sorted rows. */
    CHECK(mouse_toggle_first(list));CHECK(wait_check(list,0,1));
    {LVITEMW state={0};SIZE_T written=0;void *remote=VirtualAllocEx(process,NULL,sizeof(state),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);CHECK(remote);state.stateMask=LVIS_SELECTED|LVIS_FOCUSED;CHECK(WriteProcessMemory(process,remote,&state,sizeof(state),&written));SendMessageW(list,LVM_SETITEMSTATE,0,(LPARAM)remote);VirtualFreeEx(process,remote,0,MEM_RELEASE);}
    CHECK(select_mapping(list,1));SendMessageW(list,WM_KEYDOWN,VK_SPACE,0);SendMessageW(list,WM_KEYUP,VK_SPACE,0);CHECK(wait_check(list,1,1));
    command(client,3002);CHECK(wait_file(profile,"version=2")&&wait_file(profile,"!J=Left"));CHECK(wait_text(binding_page,L"Saved; changes not active"));
    CHECK(probe_j(1));CHECK(activate_profile(L"First"));CHECK(probe_j(0));
    /* Restore the entries; version 2 remains and the rest of this suite runs
       against the same initially enabled mapping behavior. */
    CHECK(select_mapping(list,1));SendMessageW(list,WM_KEYDOWN,VK_SPACE,0);SendMessageW(list,WM_KEYUP,VK_SPACE,0);CHECK(wait_check(list,1,2));
    CHECK(mouse_toggle_first(list));CHECK(wait_check(list,0,2));command(client,3002);CHECK(activate_profile(L"First"));CHECK(probe_j(1));
    puts("PASS mapping enabled defaults, sorted mouse/Space toggles, dialog Confirm/Cancel, Save isolation, and enabled-only native input");fflush(stdout);
    dialog=add_dialog();CHECK(dialog);CHECK(has_text(dialog,L"Edit mapping"));CHECK(wait_text(dialog,L"Trigger")&&wait_text(dialog,L"Type")&&wait_text(dialog,L"Record trigger"));
    CHECK(SendMessageW(GetDlgItem(dialog,3107),BM_GETCHECK,0,0)==BST_CHECKED);
    input=first_edit(dialog);scope=FindWindowExW(dialog,NULL,L"COMBOBOX",NULL);action=FindWindowExW(dialog,scope,L"COMBOBOX",NULL);CHECK(input&&scope&&action);
    CHECK(compact_control(input)&&compact_control(GetDlgItem(dialog,3100)));
    {INPUT keys[2]={0};keys[0].type=INPUT_KEYBOARD;keys[0].ki.wVk='J';keys[1]=keys[0];keys[1].ki.dwFlags=KEYEVENTF_KEYUP;CHECK(GetForegroundWindow()==dialog);CHECK(SendInput(2,keys,sizeof(INPUT))==2);CHECK(wait_letter(input));}
    set_value(input,L"Z");SendMessageW(scope,CB_SETCURSEL,1,0);SendMessageW(action,CB_SETCURSEL,15,0);command(dialog,3104);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(wait_text(binding_page,L"Unsaved changes")&&!file_contains(profile,"Z=date"));CHECK(SendMessageW(list,LVM_GETITEMCOUNT,0,0)==4);
    CHECK(named_action(3013,L"UpdatedFirst"));command(client,3002);CHECK(wait_file(profile,"Z=date")&&wait_text(binding_page,L"Active: First"));CHECK(activate_profile(L"UpdatedFirst"));
    CHECK(file_contains(backup,"name=First")&&file_contains(profile,"; keep this movement comment"));
    CHECK(named_action(3013,L"Unsaved"));CHECK(select_profile(L"Other"));file=wait_window(L"#32770");CHECK(file);command(file,IDCANCEL);CHECK(dismissed(file));file=NULL;
    CHECK(wait_text(binding_page,L"Active: UpdatedFirst")&&wait_text(binding_page,L"Unsaved changes"));
    CHECK(select_profile(L"Other"));file=wait_window(L"#32770");CHECK(file);command(file,IDNO);CHECK(dismissed(file));file=NULL;CHECK(wait_text(binding_page,L"Editing: Other")&&wait_text(binding_page,L"Active: UpdatedFirst"));CHECK(activate_profile(L"Other"));
    CHECK(!file_contains(profile,"name=Unsaved"));CHECK(select_profile(L"UpdatedFirst"));CHECK(activate_profile(L"UpdatedFirst"));
    CHECK(named_action(3013,L"SavedBeforeSwitch"));CHECK(select_profile(L"Other"));file=wait_window(L"#32770");CHECK(file);command(file,IDYES);CHECK(dismissed(file));file=NULL;
    CHECK(wait_file(profile,"name=SavedBeforeSwitch")&&wait_text(binding_page,L"Editing: Other")&&wait_text(binding_page,L"Active: UpdatedFirst"));CHECK(activate_profile(L"Other"));CHECK(select_profile(L"SavedBeforeSwitch"));CHECK(activate_profile(L"SavedBeforeSwitch"));
    puts("PASS mapping editing, Rename, current Save activation, and dropdown Save/Don't Save/Cancel switching");fflush(stdout);
    CHECK(named_action(3013,L"ReadOnlyDraft"));original_attributes=GetFileAttributesW(profile);CHECK(original_attributes!=INVALID_FILE_ATTRIBUTES);CHECK(SetFileAttributesW(profile,original_attributes|FILE_ATTRIBUTE_READONLY));
    CHECK(select_profile(L"Other"));file=wait_window(L"#32770");CHECK(file);command(file,IDYES);CHECK(dismissed(file));file=NULL;
    file=wait_window(L"#32770");CHECK(file);CHECK(wait_text(file,L"read-only"));command(file,IDOK);CHECK(dismissed(file));file=NULL;
    CHECK(wait_text(binding_page,L"Active: SavedBeforeSwitch")&&wait_text(binding_page,L"Unsaved changes"));CHECK(SetFileAttributesW(profile,original_attributes));original_attributes=INVALID_FILE_ATTRIBUTES;
    CHECK(named_action(3013,L"SavedBeforeSwitch"));CHECK(wait_text(binding_page,L"Saved; active"));
    command(client,3013);dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);set_value(GetDlgItem(dialog,3200),L"Other");command(dialog,3201);dialog=NULL;
    file=wait_window(L"#32770");CHECK(file);CHECK(wait_text(file,L"already exists"));command(file,IDOK);CHECK(dismissed(file));file=NULL;
    dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);command(dialog,3202);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(wait_text(binding_page,L"Saved; active"));puts("PASS failed Save blocks dropdown switching and duplicate Rename preserves the profile");fflush(stdout);
    dialog=add_dialog();CHECK(dialog);CHECK(chord_input(dialog));CHECK(wait_text(client,L"Disabled"));CHECK(chord_input(dialog));CHECK(wait_text(client,L"Enabled"));
    command(dialog,3100);CHECK(wait_text(dialog,L"Recording. Press"));CHECK(chord_input(dialog));CHECK(wait_text(first_edit(dialog),L"LControlKey+LShiftKey+F11"));
    CHECK(has_text(client,L"Enabled"));CHECK(has_text(first_edit(dialog),L"LControlKey+LShiftKey+F11"));command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;
    dialog=add_dialog();CHECK(dialog);command(dialog,3100);CHECK(wait_text(dialog,L"Recording. Press"));CHECK(missed_release_recovery(dialog,L"Enabled"));command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;
    command(client,1002);CHECK(wait_text(client,L"Disabled"));dialog=add_dialog();CHECK(dialog);command(dialog,3100);CHECK(wait_text(dialog,L"Recording. Press"));CHECK(chord_input(dialog));CHECK(wait_text(first_edit(dialog),L"LControlKey+LShiftKey+F11"));CHECK(has_text(client,L"Disabled"));command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;
    dialog=add_dialog();CHECK(dialog);command(dialog,3100);CHECK(wait_text(dialog,L"Recording. Press"));CHECK(missed_release_recovery(dialog,L"Disabled"));command(dialog,3105);CHECK(dismissed(dialog));dialog=NULL;
    puts("PASS renamed mapping recorder retains modifier sides, global toggle bypass, desktop recovery, and enabled/disabled target");fflush(stdout);
    dialog=add_dialog();CHECK(dialog);input=first_edit(dialog);scope=FindWindowExW(dialog,NULL,L"COMBOBOX",NULL);action=FindWindowExW(dialog,scope,L"COMBOBOX",NULL);
    set_value(input,L"Q");SendMessageW(scope,CB_SETCURSEL,2,0);SendMessageW(action,CB_SETCURSEL,15,0);command(dialog,3104);CHECK(dismissed(dialog));dialog=NULL;
    CHECK(named_action(3004,L"Clone"));file=wait_window(L"#32770");CHECK(file);command(file,IDNO);CHECK(dismissed(file));file=NULL;
    CHECK(SendMessageW(list,LVM_GETITEMCOUNT,0,0)==5);command(client,3002);CHECK(wait_file(clone,"Q=date")&&wait_text(binding_page,L"Saved; inactive"));CHECK(!file_contains(profile,"Q=date"));
    CHECK(wait_text(binding_page,L"Active: SavedBeforeSwitch")&&has_profile(L"Clone"));CHECK(IsWindowEnabled(GetDlgItem(binding_page,3005)));
    CHECK(choose_delete(L"Clone"));file=wait_window(L"#32770");CHECK(file);command(file,IDNO);CHECK(dismissed(file));file=NULL;CHECK(file_contains(clone,"Z=date"));
    CHECK(choose_delete(L"Clone"));file=wait_window(L"#32770");CHECK(file);command(file,IDYES);CHECK(dismissed(file));file=NULL;pump(100);CHECK(GetFileAttributesW(clone)==INVALID_FILE_ATTRIBUTES&&!has_profile(L"Clone"));
    CHECK(wait_text(binding_page,L"Active: SavedBeforeSwitch")&&file_contains(profile,"Z=date"));
    CHECK(named_action(3000,L"Empty"));CHECK(SendMessageW(list,LVM_GETITEMCOUNT,0,0)==0);command(client,3002);CHECK(wait_file(empty,"name=Empty")&&wait_text(binding_page,L"Saved; inactive"));
    CHECK(has_profile(L"Empty"));CHECK(select_profile(L"Empty"));CHECK(activate_profile(L"Empty")&&has_text(client,L"Disabled"));CHECK(IsWindowEnabled(GetDlgItem(binding_page,3005)));
    CHECK(named_action(3013,L"Empty renamed"));command(client,3002);CHECK(wait_file(empty,"name=Empty renamed")&&wait_text(binding_page,L"Active: Empty"));CHECK(activate_profile(L"Empty renamed"));CHECK(has_text(client,L"Disabled"));
    puts("PASS Copy/New save into default directory, inactive Delete confirmation, and disabled dropdown/current Save activation");fflush(stdout);
    command(client,3003);file=wait_window(L"#32770");CHECK(file);CHECK(choose_file(file,external));CHECK(dismissed(file));file=NULL;
    CHECK(wait_file(external,"name=Empty renamed"));CHECK(wait_text(binding_page,L"Saved; inactive"));CHECK(IsWindowEnabled(GetDlgItem(binding_page,3005)));
    CHECK(named_action(3013,L"ExternalOnly"));command(client,3002);CHECK(wait_file(external,"name=ExternalOnly"));CHECK(!has_profile(L"ExternalOnly"));
    command(client,3005);dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);CHECK(SendMessageW(GetDlgItem(dialog,3200),CB_FINDSTRINGEXACT,(WPARAM)-1,(LPARAM)L"ExternalOnly")==CB_ERR);command(dialog,3202);CHECK(dismissed(dialog));dialog=NULL;CHECK(SendMessageW(GetDlgItem(binding_page,3001),CB_GETCOUNT,0,0)==3);
    command(client,3006);file=wait_window(L"#32770");CHECK(file);CHECK(choose_file(file,external));CHECK(dismissed(file));file=NULL;
    dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);set_value(GetDlgItem(dialog,3200),L"Imported");command(dialog,3201);CHECK(dismissed(dialog));dialog=NULL;
    command(client,3002);CHECK(wait_file(imported,"name=Imported")&&has_profile(L"Imported"));CHECK(file_contains(external,"name=ExternalOnly"));
    CHECK(select_profile(L"Imported"));CHECK(activate_profile(L"Imported"));
    command(client,3006);file=wait_window(L"#32770");CHECK(file);CHECK(choose_file(file,invalid));CHECK(dismissed(file));file=NULL;
    file=wait_window(L"#32770");CHECK(file);CHECK(wait_text(file,L"version=1"));command(file,IDOK);CHECK(dismissed(file));file=NULL;CHECK(wait_text(binding_page,L"Active: Imported"));CHECK(file_contains(invalid,"version=99"));
    command(client,3006);file=wait_window(L"#32770");CHECK(file);CHECK(choose_file(file,legacy));CHECK(dismissed(file));file=NULL;
    dialog=wait_window(L"TypingProfileNameDialog");CHECK(dialog);set_value(GetDlgItem(dialog,3200),L"Legacy copy");command(dialog,3201);CHECK(dismissed(dialog));dialog=NULL;
    command(client,3002);CHECK(wait_file(legacy_copy,"name=Legacy copy")&&has_profile(L"Legacy copy"));CHECK(file_contains(legacy,"CapsLock=mode"));
    puts("PASS external Save As excluded from dropdown, profile/legacy Import preserves sources, invalid profile never falls back to legacy");fflush(stdout);
    CHECK(select_profile(L"Imported"));CHECK(activate_profile(L"Imported"));CHECK(named_action(3013,L"CloseDraft"));
    CHECK(named_action(3000,L"Canceled new"));file=wait_window(L"#32770");CHECK(file);command(file,IDCANCEL);CHECK(dismissed(file));file=NULL;CHECK(wait_text(binding_page,L"Editing: CloseDraft")&&wait_text(binding_page,L"Unsaved changes"));
    PostMessageW(client,WM_CLOSE,0,0);file=wait_window(L"#32770");CHECK(file);command(file,IDCANCEL);CHECK(dismissed(file));file=NULL;CHECK(IsWindowVisible(client));
    PostMessageW(client,WM_CLOSE,0,0);file=wait_window(L"#32770");CHECK(file);command(file,IDNO);CHECK(dismissed(file));file=NULL;pump(100);CHECK(!IsWindowVisible(client));command(client,1004);CHECK(wait_text(binding_page,L"Saved; active"));
    CHECK(named_action(3013,L"ExitSaved"));command(client,1003);file=wait_window(L"#32770");CHECK(file);command(file,IDCANCEL);CHECK(dismissed(file));file=NULL;CHECK(WaitForSingleObject(process,0)==WAIT_TIMEOUT);
    command(client,1003);file=wait_window(L"#32770");CHECK(file);command(file,IDYES);CHECK(dismissed(file));file=NULL;CHECK(wait_file(imported,"name=ExitSaved"));CHECK(WaitForSingleObject(process,5000)==WAIT_OBJECT_0);
    CHECK(file_contains(config,"mode_auto_exit=off")&&file_contains(config,"; application settings stay independent"));
    puts("PASS Close/Exit unsaved prompts preserve drafts, saved baseline, and independent application settings");fflush(stdout);
cleanup:
    if(client&&process&&WaitForSingleObject(process,0)==WAIT_TIMEOUT){
        struct search named={L"TypingProfileNameDialog",NULL};EnumWindows(find_window,(LPARAM)&named);if(named.result){PostMessageW(named.result,WM_CLOSE,0,0);pump(50);}
        struct search mapping={L"TypingBindingEditor",NULL};EnumWindows(find_window,(LPARAM)&mapping);if(mapping.result){command(mapping.result,3105);pump(50);}
    }
    if(original_attributes!=INVALID_FILE_ATTRIBUTES)SetFileAttributesW(profile,original_attributes);
    if(dialog&&IsWindow(dialog)){command(dialog,3202);command(dialog,3105);}
    if(file&&IsWindow(file))command(file,IDCANCEL);
    if(client&&process&&WaitForSingleObject(process,0)==WAIT_TIMEOUT){command(client,1003);pump(100);HWND popup=wait_window(L"#32770");if(popup)command(popup,IDNO);}
    if(process){if(WaitForSingleObject(process,5000)!=WAIT_OBJECT_0){fwprintf(stderr,L"Test client did not exit cleanly; PID %lu\n",pid);failed=1;}else{GetExitCodeProcess(process,&exit_code);if(exit_code!=0)failed=1;}CloseHandle(process);}
    if(child.hThread)CloseHandle(child.hThread);if(controller)DestroyWindow(controller);return failed?1:0;
}
