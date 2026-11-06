OUTPUT_ARCH(riscv)
ENTRY(_start)

PHDRS {
  code PT_LOAD FLAGS(5);
  constants PT_LOAD FLAGS(4);
  data PT_LOAD FLAGS(6);
}

SECTIONS {
  . = 0x10000;
  .text : { KEEP(*(.text.entry)) *(.text .text.*) } :code
  . = ALIGN(4096);
  .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } :constants
  . = ALIGN(4096);
  .data : {
    PROVIDE(__global_pointer$ = . + 0x800);
    *(.data .data.* .sdata .sdata.*)
  } :data
  .bss (NOLOAD) : { *(.bss .bss.* .sbss .sbss.*) *(COMMON) } :data
}
