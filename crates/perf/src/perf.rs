use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Importance {
    Critical,
    Important,
    #[default]
    Average,
    Iffy,
    Fluff,
}

impl fmt::Display for Importance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Importance::Critical => write!(f, "Critical"),
            Importance::Important => write!(f, "Important"),
            Importance::Average => write!(f, "Average"),
            Importance::Iffy => write!(f, "Iffy"),
            Importance::Fluff => write!(f, "Fluff"),
        }
    }
}

pub mod consts {
    pub const SUF_NORMAL: &str = ".perf";
    pub const SUF_MDATA: &str = ".perfm";
    pub const ITER_ENV_VAR: &str = "PERF_ITERATIONS";
    pub const MDATA_LINE_PREF: &str = "PERF_MDATA";
    pub const ITER_COUNT_LINE_NAME: &str = "iterations";
    pub const WEIGHT_LINE_NAME: &str = "weight";
    pub const IMPORTANCE_LINE_NAME: &str = "importance";
    pub const VERSION_LINE_NAME: &str = "version";
    pub const MDATA_VER: u32 = 1;
    pub const WEIGHT_DEFAULT: u32 = 1;
}
