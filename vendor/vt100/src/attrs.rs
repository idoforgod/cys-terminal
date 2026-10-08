use crate::term::BufWrite as _;

/// Represents a foreground or background color for cells.
#[derive(Eq, PartialEq, Debug, Copy, Clone)]
pub enum Color {
    /// The default terminal color.
    Default,

    /// An indexed terminal color.
    Idx(u8),

    /// An RGB terminal color. The parameters are (red, green, blue).
    Rgb(u8, u8, u8),
}

impl Default for Color {
    fn default() -> Self {
        Self::Default
    }
}

const TEXT_MODE_BOLD: u8 = 0b0000_0001;
const TEXT_MODE_ITALIC: u8 = 0b0000_0010;
const TEXT_MODE_UNDERLINE: u8 = 0b0000_0100;
const TEXT_MODE_INVERSE: u8 = 0b0000_1000;
// cys 패치(0.14.47): 흐림(SGR 2) 비트. **읽기 전용 재료**다 — 칸 비교(아래 PartialEq)와 다시 그리기 출력
// (write_escape_code_diff)에는 넣지 않는다. 재적용 경로 = vendor/vt100/CYS-PATCHES.md.
const TEXT_MODE_DIM: u8 = 0b0001_0000;

#[derive(Default, Clone, Copy, Debug)]
pub struct Attrs {
    pub fgcolor: Color,
    pub bgcolor: Color,
    pub mode: u8,
}

// cys 패치(0.14.47): 원본의 파생 비교를 손 구현으로 바꿨다 — **흐림 비트만** 가리고 나머지(전경·배경·굵게·기울임·밑줄·반전)는
// 원본과 같이 본다. 흐림이 비교에 들어가면 contents_formatted/contents_diff 의 바이트가 바뀐다(흐림을 쓸 줄 모르는 출력부가
// 경계마다 속성 초기화를 더 찍는다).
impl PartialEq for Attrs {
    fn eq(&self, other: &Self) -> bool {
        self.fgcolor == other.fgcolor
            && self.bgcolor == other.bgcolor
            && (self.mode & !TEXT_MODE_DIM) == (other.mode & !TEXT_MODE_DIM)
    }
}

impl Eq for Attrs {}

impl Attrs {
    // cys 패치(0.14.47): 흐림 읽기·쓰기.
    pub fn dim(&self) -> bool {
        self.mode & TEXT_MODE_DIM != 0
    }

    pub fn set_dim(&mut self, dim: bool) {
        if dim {
            self.mode |= TEXT_MODE_DIM;
        } else {
            self.mode &= !TEXT_MODE_DIM;
        }
    }

    pub fn bold(&self) -> bool {
        self.mode & TEXT_MODE_BOLD != 0
    }

    pub fn set_bold(&mut self, bold: bool) {
        if bold {
            self.mode |= TEXT_MODE_BOLD;
        } else {
            self.mode &= !TEXT_MODE_BOLD;
        }
    }

    pub fn italic(&self) -> bool {
        self.mode & TEXT_MODE_ITALIC != 0
    }

    pub fn set_italic(&mut self, italic: bool) {
        if italic {
            self.mode |= TEXT_MODE_ITALIC;
        } else {
            self.mode &= !TEXT_MODE_ITALIC;
        }
    }

    pub fn underline(&self) -> bool {
        self.mode & TEXT_MODE_UNDERLINE != 0
    }

    pub fn set_underline(&mut self, underline: bool) {
        if underline {
            self.mode |= TEXT_MODE_UNDERLINE;
        } else {
            self.mode &= !TEXT_MODE_UNDERLINE;
        }
    }

    pub fn inverse(&self) -> bool {
        self.mode & TEXT_MODE_INVERSE != 0
    }

    pub fn set_inverse(&mut self, inverse: bool) {
        if inverse {
            self.mode |= TEXT_MODE_INVERSE;
        } else {
            self.mode &= !TEXT_MODE_INVERSE;
        }
    }

    pub fn write_escape_code_diff(
        &self,
        contents: &mut Vec<u8>,
        other: &Self,
    ) {
        if self != other && self == &Self::default() {
            crate::term::ClearAttrs::default().write_buf(contents);
            return;
        }

        let attrs = crate::term::Attrs::default();

        let attrs = if self.fgcolor == other.fgcolor {
            attrs
        } else {
            attrs.fgcolor(self.fgcolor)
        };
        let attrs = if self.bgcolor == other.bgcolor {
            attrs
        } else {
            attrs.bgcolor(self.bgcolor)
        };
        let attrs = if self.bold() == other.bold() {
            attrs
        } else {
            attrs.bold(self.bold())
        };
        let attrs = if self.italic() == other.italic() {
            attrs
        } else {
            attrs.italic(self.italic())
        };
        let attrs = if self.underline() == other.underline() {
            attrs
        } else {
            attrs.underline(self.underline())
        };
        let attrs = if self.inverse() == other.inverse() {
            attrs
        } else {
            attrs.inverse(self.inverse())
        };

        attrs.write_buf(contents);
    }
}
