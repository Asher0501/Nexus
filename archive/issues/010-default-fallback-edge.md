# Issue 010: 默认 fallback 边（已废弃）

**状态**: 已废弃

根据**声明完备性原则**，引擎不弥补声明的缺口。未匹配的 exit_reason 属于声明不完备，引擎应该崩溃退出，而不是添加默认 fallback 边。

详见 DESIGN_PHILOSOPHY.md §0 的声明完备性原则。