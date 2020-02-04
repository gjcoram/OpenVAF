use crate::compact_arena::{Idx16, Idx8};

macro_rules! impl_id_type {
    ($name:ident in $container:ident::$sub_container:ident -> $type:ty) => {
        impl<'tag> ::std::ops::Index<$name<'tag>> for $container<'tag> {
            type Output = $type;
            fn index(&self, index: $name<'tag>) -> &Self::Output {
                &self.$sub_container[index.0]
            }
        }
        impl<'tag> ::std::ops::Index<Range<$name<'tag>>> for $container<'tag> {
            type Output = [$type];
            fn index(&self, range: Range<$name<'tag>>) -> &Self::Output {
                let range = $crate::compact_arena::SafeRange::new(range.start.0, range.end.0);
                &self.$sub_container[range]
            }
        }
        impl<'tag> ::std::ops::Index<$crate::compact_arena::SafeRange<$name<'tag>>>
            for $container<'tag>
        {
            type Output = [$type];
            fn index(&self, range: $crate::compact_arena::SafeRange<$name<'tag>>) -> &Self::Output {
                let range = unsafe {
                    $crate::compact_arena::SafeRange::new(range.get_start().0, range.get_end().0)
                };
                &self.$sub_container[range]
            }
        }

        impl<'tag> ::std::ops::IndexMut<$name<'tag>> for $container<'tag> {
            fn index_mut(&mut self, index: $name<'tag>) -> &mut Self::Output {
                &mut self.$sub_container[index.0]
            }
        }
        impl<'tag> ::std::ops::IndexMut<Range<$name<'tag>>> for $container<'tag> {
            fn index_mut(&mut self, range: Range<$name<'tag>>) -> &mut Self::Output {
                let range = $crate::compact_arena::SafeRange::new(range.start.0, range.end.0);
                &mut self.$sub_container[range]
            }
        }
        impl<'tag> ::std::ops::IndexMut<$crate::compact_arena::SafeRange<$name<'tag>>>
            for $container<'tag>
        {
            fn index_mut(
                &mut self,
                range: $crate::compact_arena::SafeRange<$name<'tag>>,
            ) -> &mut Self::Output {
                let range = unsafe {
                    $crate::compact_arena::SafeRange::new(range.get_start().0, range.get_end().0)
                };
                &mut self.$sub_container[range]
            }
        }
        impl<'tag> $crate::util::Push<$type> for $container<'tag> {
            type Key = $name<'tag>;
            fn push(&mut self, val: $type) -> Self::Key {
                $name(self.$sub_container.add(val))
            }
        }
        impl<'tag> $crate::util::SafeRangeCreation<$name<'tag>> for $container<'tag> {
            fn range_to_end(&self, from: $name<'tag>) -> SafeRange<$name<'tag>> {
                let range = self.$sub_container.range_to_end(from.0);
                unsafe {
                    $crate::compact_arena::SafeRange::new(
                        $name(range.get_start()),
                        $name(range.get_end()),
                    )
                }
            }
            fn empty_range_from_end(&self) -> SafeRange<$name<'tag>> {
                let range = self.$sub_container.empty_range_from_end();
                unsafe {
                    $crate::compact_arena::SafeRange::new(
                        $name(range.get_start()),
                        $name(range.get_end()),
                    )
                }
            }
            fn extend_range_to_end(&self, range: SafeRange<$name<'tag>>) -> SafeRange<$name<'tag>> {
                let range = self.$sub_container.extend_range_to_end(range.into());
                unsafe {
                    $crate::compact_arena::SafeRange::new(
                        $name(range.get_start()),
                        $name(range.get_end()),
                    )
                }
            }
            fn full_range(&self) -> SafeRange<$name<'tag>> {
                let range = self.$sub_container.full_range();
                unsafe {
                    $crate::compact_arena::SafeRange::new(
                        $name(range.get_start()),
                        $name(range.get_end()),
                    )
                }
            }
        }
    };
}

macro_rules! id_type {
    ($name:ident($type:ident)) => {
        #[derive(Copy, Clone, PartialOrd, PartialEq, Eq, Debug)]
        #[repr(transparent)]
        pub struct $name<'tag>($type<'tag>);

        impl<'tag> $crate::util::Step for $name<'tag> {
            unsafe fn step(&mut self) {
                self.0.add(1)
            }
        }
        impl<'tag> ::std::convert::Into<$type<'tag>> for $name<'tag> {
            fn into(self) -> $type<'tag> {
                self.0
            }
        }
        impl<'tag> ::std::convert::Into<$crate::compact_arena::SafeRange<$type<'tag>>>
            for $crate::compact_arena::SafeRange<$name<'tag>>
        {
            fn into(self) -> $crate::compact_arena::SafeRange<$type<'tag>> {
                $crate::compact_arena::SafeRange::new(
                    unsafe { self.get_start() }.0,
                    unsafe { self.get_end() }.0,
                )
            }
        }
    };
}

#[macro_use]
pub mod ast;
#[macro_use]
pub mod hir;

id_type!(BranchId(Idx8));
id_type!(NetId(Idx16));
id_type!(PortId(Idx8));
id_type!(VariableId(Idx16));
id_type!(ModuleId(Idx8));
id_type!(FunctionId(Idx8));
id_type!(DisciplineId(Idx8));
id_type!(ExpressionId(Idx16));
id_type!(BlockId(Idx8));
id_type!(AttributeId(Idx16));
id_type!(StatementId(Idx16));
id_type!(NatureId(Idx8));