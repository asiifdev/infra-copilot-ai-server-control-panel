//! Closed enums stored as text. The SQL `CHECK` constraints list the same values.

macro_rules! text_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vdoc])* $variant),+
        }

        impl $name {
            /// Stored text form.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            /// Parses the stored text form.
            ///
            /// # Errors
            /// [`crate::Error::Invalid`] for an unknown value.
            pub fn parse(text: &str) -> crate::Result<Self> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(crate::Error::Invalid(stringify!($name))),
                }
            }
        }
    };
}
