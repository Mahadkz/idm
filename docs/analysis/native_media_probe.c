/* Invoke selected original media exports only with disposable synthetic files.
 * Calling convention/argument count must be checked against the target first. */
#include <windows.h>
#include <stdio.h>
#include <wchar.h>
typedef unsigned char (__cdecl *ConvertTwo)(const wchar_t *, const wchar_t *);
typedef unsigned char (__cdecl *MuxThree)(const wchar_t *, const wchar_t *, const wchar_t *);
int wmain(int argc,wchar_t **argv) {
 if(argc<5)return 2;
 HMODULE library=LoadLibraryW(argv[1]);
 if(!library){printf("{\"load_error\":%lu}\n",GetLastError());return 3;}
 unsigned char result=0;
 if(!wcscmp(argv[2],L"convert")) {
  ConvertTwo call=(ConvertTwo)GetProcAddress(library,"ConvertTsToMp4");if(!call)return 4;
  result=call(argv[3],argv[4]);
 } else if(!wcscmp(argv[2],L"mux")&&argc==6) {
  MuxThree call=(MuxThree)GetProcAddress(library,"MuxTSandTStoMP4");if(!call)return 4;
  result=call(argv[3],argv[4],argv[5]);
 } else return 2;
 printf("{\"return_byte\":%u}\n",result);FreeLibrary(library);return 0;
}
