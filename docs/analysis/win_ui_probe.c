/* Original audit helper: inspect/control only the owned private Wine session.
 * No proprietary code is included; this is not application implementation. */
#ifndef UNICODE
#define UNICODE
#endif
#define _UNICODE
#include <windows.h>
#include <commctrl.h>
#include <stdio.h>
#include <stdlib.h>
#include <wchar.h>

static void quoted(const wchar_t *wide) {
 char text[16384]; int n=WideCharToMultiByte(CP_UTF8,0,wide,-1,text,sizeof(text),NULL,NULL);
 putchar('"');
 if(n>0) for(int i=0;i<n-1;i++) {unsigned char c=text[i];if(c=='"'||c=='\\')putchar('\\');if(c<32)printf("\\u%04x",c);else putchar(c);}
 putchar('"');
}
static BOOL CALLBACK emit(HWND h,LPARAM ignored) {
 wchar_t label[2048]={0},klass[128]={0};RECT r;DWORD pid=0;
 SendMessageTimeoutW(h,WM_GETTEXT,2048,(LPARAM)label,SMTO_ABORTIFHUNG,1000,NULL);GetClassNameW(h,klass,128);GetWindowRect(h,&r);GetWindowThreadProcessId(h,&pid);
 printf("{\"hwnd\":\"%p\",\"pid\":%lu,\"parent\":\"%p\",\"id\":%d,\"class\":",h,pid,GetParent(h),GetDlgCtrlID(h));quoted(klass);
 printf(",\"text\":");quoted(label);
 printf(",\"visible\":%s,\"enabled\":%s,\"rect\":[%ld,%ld,%ld,%ld],\"style\":%lu",IsWindowVisible(h)?"true":"false",IsWindowEnabled(h)?"true":"false",r.left,r.top,r.right,r.bottom,(unsigned long)GetWindowLongW(h,GWL_STYLE));
 if(!wcscmp(klass,L"Button"))printf(",\"checked\":%lld",(long long)SendMessageW(h,BM_GETCHECK,0,0));
 if(!wcscmp(klass,L"ComboBox")) {LRESULT count=SendMessageW(h,CB_GETCOUNT,0,0);printf(",\"selected\":%lld,\"items\":[",(long long)SendMessageW(h,CB_GETCURSEL,0,0));for(int i=0;i<count&&i<100;i++){wchar_t value[2048]={0};if(i)putchar(',');if(SendMessageW(h,CB_GETLBTEXTLEN,i,0)<2048){SendMessageW(h,CB_GETLBTEXT,i,(LPARAM)value);quoted(value);}else quoted(L"<too-long>");}putchar(']');}
 if(!wcscmp(klass,L"SysListView32")) printf(",\"item_count\":%lld",(long long)SendMessageW(h,LVM_GETITEMCOUNT,0,0));
 printf("}\n");return TRUE;
}
static BOOL CALLBACK top(HWND h,LPARAM filter) {if(IsWindowVisible(h)){emit(h,0);EnumChildWindows(h,emit,0);}return TRUE;}
static void menu(HMENU m,int depth){for(int i=0;i<GetMenuItemCount(m);i++){wchar_t text[1024]={0};MENUITEMINFOW item={0};item.cbSize=sizeof(item);item.fMask=MIIM_ID|MIIM_STRING|MIIM_STATE|MIIM_SUBMENU;item.dwTypeData=text;item.cch=1024;GetMenuItemInfoW(m,i,TRUE,&item);printf("{\"depth\":%d,\"id\":%u,\"state\":%u,\"text\":",depth,item.wID,item.fState);quoted(text);puts("}");if(item.hSubMenu)menu(item.hSubMenu,depth+1);}}
static void list(HWND h) {
 DWORD pid;GetWindowThreadProcessId(h,&pid);HANDLE process=OpenProcess(PROCESS_VM_OPERATION|PROCESS_VM_READ|PROCESS_VM_WRITE,FALSE,pid);
 if(!process)return;void *remote=VirtualAllocEx(process,NULL,8192,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!remote){CloseHandle(process);return;}
 int count=(int)SendMessageW(h,LVM_GETITEMCOUNT,0,0);int columns=(int)SendMessageW((HWND)SendMessageW(h,LVM_GETHEADER,0,0),HDM_GETITEMCOUNT,0,0);
 for(int row=0;row<count;row++){printf("{\"row\":%d,\"cells\":[",row);for(int col=0;col<columns;col++){LVITEMW item={0};wchar_t text[2048]={0};item.iSubItem=col;item.pszText=(wchar_t*)((char*)remote+1024);item.cchTextMax=2048;WriteProcessMemory(process,remote,&item,sizeof(item),NULL);WriteProcessMemory(process,item.pszText,text,sizeof(text),NULL);SendMessageW(h,LVM_GETITEMTEXTW,row,(LPARAM)remote);ReadProcessMemory(process,item.pszText,text,sizeof(text),NULL);if(col)putchar(',');quoted(text);}puts("]}");}
 VirtualFreeEx(process,remote,0,MEM_RELEASE);CloseHandle(process);
}
static void treeItem(HWND h,HANDLE process,void *remote,HTREEITEM item,int depth) {
 for(;item;item=(HTREEITEM)SendMessageW(h,TVM_GETNEXTITEM,TVGN_NEXT,(LPARAM)item)) {
  TVITEMW value={0};wchar_t text[1024]={0};value.mask=TVIF_TEXT|TVIF_STATE;value.hItem=item;value.pszText=(wchar_t*)((char*)remote+1024);value.cchTextMax=1024;value.stateMask=0xffffffff;
  WriteProcessMemory(process,remote,&value,sizeof(value),NULL);WriteProcessMemory(process,value.pszText,text,sizeof(text),NULL);SendMessageW(h,TVM_GETITEMW,0,(LPARAM)remote);ReadProcessMemory(process,value.pszText,text,sizeof(text),NULL);
  printf("{\"item\":\"%p\",\"depth\":%d,\"text\":",item,depth);quoted(text);puts("}");
  treeItem(h,process,remote,(HTREEITEM)SendMessageW(h,TVM_GETNEXTITEM,TVGN_CHILD,(LPARAM)item),depth+1);
 }
}
static void tree(HWND h){DWORD pid;GetWindowThreadProcessId(h,&pid);HANDLE process=OpenProcess(PROCESS_VM_OPERATION|PROCESS_VM_READ|PROCESS_VM_WRITE,FALSE,pid);if(!process)return;void *remote=VirtualAllocEx(process,NULL,8192,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(remote){treeItem(h,process,remote,(HTREEITEM)SendMessageW(h,TVM_GETNEXTITEM,TVGN_ROOT,0),0);VirtualFreeEx(process,remote,0,MEM_RELEASE);}CloseHandle(process);}
static void listSelect(HWND h,int row){DWORD pid;GetWindowThreadProcessId(h,&pid);HANDLE process=OpenProcess(PROCESS_VM_OPERATION|PROCESS_VM_READ|PROCESS_VM_WRITE,FALSE,pid);if(!process)return;void *remote=VirtualAllocEx(process,NULL,sizeof(LVITEMW),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(remote){LVITEMW item={0};item.state=LVIS_SELECTED|LVIS_FOCUSED;item.stateMask=LVIS_SELECTED|LVIS_FOCUSED;WriteProcessMemory(process,remote,&item,sizeof(item),NULL);SendMessageW(h,LVM_SETITEMSTATE,row,(LPARAM)remote);VirtualFreeEx(process,remote,0,MEM_RELEASE);}CloseHandle(process);}
int wmain(int argc,wchar_t **argv) {
 if(argc<2)return 2;
 if(!wcscmp(argv[1],L"dump")){EnumWindows(top,0);return 0;}
 if(argc<3)return 2;HWND h=(HWND)(ULONG_PTR)wcstoull(argv[2],NULL,16);if(!IsWindow(h))return 3;
 if(!wcscmp(argv[1],L"menu")){menu(GetMenu(h),0);return 0;}
 if(!wcscmp(argv[1],L"list")){list(h);return 0;}
 if(!wcscmp(argv[1],L"tree")){tree(h);return 0;}
 if(!wcscmp(argv[1],L"click")){PostMessageW(h,BM_CLICK,0,0);return 0;}
 if(!wcscmp(argv[1],L"close")){PostMessageW(h,WM_CLOSE,0,0);return 0;}
 if(argc<4)return 2;
 if(!wcscmp(argv[1],L"tree-select")){SendMessageW(h,TVM_SELECTITEM,TVGN_CARET,(LPARAM)(ULONG_PTR)wcstoull(argv[3],NULL,16));return 0;}
 if(!wcscmp(argv[1],L"list-select")){listSelect(h,wcstoul(argv[3],NULL,10));return 0;}
 if(!wcscmp(argv[1],L"text")){SendMessageW(h,WM_SETTEXT,0,(LPARAM)argv[3]);return 0;}
 if(!wcscmp(argv[1],L"command")){PostMessageW(h,WM_COMMAND,wcstoul(argv[3],NULL,10),0);return 0;}
 if(!wcscmp(argv[1],L"select")){SendMessageW(h,CB_SETCURSEL,wcstoul(argv[3],NULL,10),0);SendMessageW(GetParent(h),WM_COMMAND,MAKEWPARAM(GetDlgCtrlID(h),CBN_SELCHANGE),(LPARAM)h);return 0;}
 return 2;
}
