.MODEL FLAT, C
.CODE

EXTERN execute_instruction_original : PTR
handle_instruction PROTO, opcode: DWORD

; EAX = [CURRENT_EXECUTION_CONTEXT]
execute_instruction_hook PROC PUBLIC
  ; Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH ECX
  PUSH EDX
  INVOKE handle_instruction, EAX
  POP EDX
  POP ECX

  ; Jump to BYOND's default do_instruction.
  MOV ECX, execute_instruction_original
  JMP ECX

;    MOV   EDI, DWORD PTR [EAX + 10h]     ; EDI = execution_context->bytecode
;    MOVZX ECX, WORD PTR [EAX + 14h]      ; ECX = execution_context->bytecode_offset
;    MOV   EDX, DWORD PTR [EDI + ECX*4h]  ; EDX = execution_context->bytecode[execution_context->bytecode_offset]
;
;    ; Give rust a chance to handle the instruction
;    PUSH EAX
;    PUSH ECX
;    PUSH EDX
;    INVOKE handle_instruction, EDX
;    CMP EAX, 0
;    JNZ instruction_handled
;
;    ; We didn't override the instruction, just continue the normal code
;    POP EDX
;    POP ECX
;    POP EAX    
;    MOV ESI, ECX ; Byond wants the bytecode_offset in ESI too
;    MOV EDX, continue_instruction_byond
;    JMP EDX
;
;    ; The instruction has been handled by rust, just loop the proc to run the next instruction
;  instruction_handled:
;    POP EDX
;    POP ECX
;    POP EAX
;    JMP do_instruction_hook
execute_instruction_hook ENDP

END