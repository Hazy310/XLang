// GUI 版入口：直接包含根包 src/main.rs（其中所有 GUI 代码由 cfg(feature = "gui") 门控，
// 本包 default feature 常开 "gui"）。避免复制维护两份源码。
include!("../../src/main.rs");
