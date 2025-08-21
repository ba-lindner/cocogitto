use std::collections::HashMap;

use tera::{dotted_pointer, to_value, try_get_value, Context, Tera, Value};

use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::{Template, ToContext};
use crate::conventional::changelog::RenderOutput;

#[derive(Debug)]
pub struct Renderer {
    tera: Tera,
    context: Context,
    name: &'static str,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(Template::default()).expect("Failed to load renderer for default template")
    }
}

impl Renderer {
    pub fn new(template: Template) -> Result<Self, tera::Error> {
        let mut tera = Tera::default();
        let content = template.content();

        tera.add_raw_template(template.name(), content)?;
        tera.register_filter("upper_first", Self::upper_first_filter);
        tera.register_filter("unscoped", Self::unscoped);

        Ok(Renderer {
            tera,
            context: Context::new(),
            name: template.name(),
        })
    }

    pub fn with_context(mut self, context: impl ToContext) -> Self {
        self.context.extend(context.to_context());
        self
    }

    pub fn render(&self, version: &Release) -> Result<RenderOutput, tera::Error> {
        let mut release = self.render_release(version)?;
        let mut version = version;
        while let Some(previous) = version.previous.as_ref() {
            release.push_str("\n- - -\n\n");
            release.push_str(self.render_release(previous)?.as_str());
            version = previous;
        }

        Ok(RenderOutput(release))
    }

    fn render_release(&self, version: &Release) -> Result<String, tera::Error> {
        let mut context = self.context.clone();
        let release_context = Context::from_serialize(version)?;
        context.extend(release_context);

        self.tera.render(self.name, &context)
    }

    // From git-cliff: https://github.com/orhun/git-cliff/blob/main/git-cliff-core/src/template.rs
    fn upper_first_filter(value: &Value, _: &HashMap<String, Value>) -> Result<Value, tera::Error> {
        let mut s = tera::try_get_value!("upper_first_filter", "value", String, value);
        let mut c = s.chars();
        s = match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        };
        Ok(tera::to_value(&s)?)
    }

    // filter commit with no scope
    fn unscoped(value: &Value, args: &HashMap<String, Value>) -> Result<Value, tera::Error> {
        let mut arr = try_get_value!("unscoped", "scope", Vec<Value>, value);
        if arr.is_empty() {
            return Ok(arr.into());
        }

        let value = args.get("value").unwrap_or(&Value::Null);

        arr = arr
            .into_iter()
            .filter(|v| {
                let val = dotted_pointer(v, "scope").unwrap_or(&Value::Null);
                if value.is_null() {
                    val == value
                } else {
                    !val.is_null()
                }
            })
            .collect::<Vec<_>>();

        Ok(to_value(arr).unwrap())
    }
}
