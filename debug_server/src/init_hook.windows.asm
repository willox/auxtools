; for i686-pc-windows-msvc
.MODEL FLAT, C
.CODE

EXTERN pre_call_new_original : PTR
handle_init_hook PROTO, value_ptr: PTR

; Pointer to the newly created object's Value is stored in ESI
pre_call_new_hook PROC PUBLIC
  PUSH EBX
  PUSH ECX
  PUSH EDX
  INVOKE handle_init_hook, ESI
  POP EDX
  POP ECX
  POP EBX

  MOV EAX, pre_call_new_original
  JMP EAX
pre_call_new_hook ENDP

END
