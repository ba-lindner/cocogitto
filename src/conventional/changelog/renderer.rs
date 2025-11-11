use std::collections::HashMap;

use tera::{dotted_pointer, to_value, try_get_value, Context, Tera, Value};

use crate::conventional::changelog::release::Release;
use crate::conventional::changelog::template::{
    Template, ToContext, MACROS_TEMPLATE, MACROS_TEMPLATE_NAME,
};

#[derive(Debug)]
pub struct Renderer {
    tera: Tera,
    context: Context,
    template: Template,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::try_new(Template::default()).expect("Failed to load renderer for default template")
    }
}

impl Renderer {
    pub fn try_new(template: Template) -> Result<Self, tera::Error> {
        let mut tera = Tera::default();

        tera.add_raw_template(MACROS_TEMPLATE_NAME, MACROS_TEMPLATE)?;
        tera.add_raw_template(template.name(), template.content())?;
        tera.register_filter("upper_first", Self::upper_first_filter);
        tera.register_filter("unscoped", Self::unscoped);
        tera.register_filter("group_by_type", Self::group_by_type);
        tera.check_macro_files()?;

        Ok(Renderer {
            tera,
            context: Context::new(),
            template,
        })
    }

    pub fn add_context(&mut self, context: impl ToContext) {
        self.context.extend(context.to_context());
    }

    pub(crate) fn render(&self, version: &[Release]) -> Result<String, tera::Error> {
        let Some(first) = version.first() else {
            return Ok(String::new());
        };
        let mut release = self.render_single(first)?;
        for version in &version[1..] {
            release.push_str("\n- - -\n\n");
            release.push_str(self.render_single(version)?.as_str());
        }

        Ok(release)
    }

    pub(crate) fn render_single(&self, version: &Release) -> Result<String, tera::Error> {
        let mut context = self.context.clone();
        context.extend(Context::from_serialize(version)?);

        self.tera.render(self.template.name(), &context)
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

    // group commits and order by type
    fn group_by_type(value: &Value, _: &HashMap<String, Value>) -> Result<Value, tera::Error> {
        let arr = try_get_value!("group_by_type", "type", Vec<Value>, value);
        let mut map = HashMap::new();

        for v in arr {
            let val = dotted_pointer(&v, "type").unwrap_or(&Value::Null);
            let val = val.as_str().unwrap_or_default();
            let val = val.to_string();

            let entry = map.entry(val).or_insert_with(Vec::new);
            entry.push(v);
        }

        // Sort the output by keys, checking the sort_order on the commit
        let mut keys = map.keys().collect::<Vec<_>>();
        keys.sort_by(|a, b| {
            let a = a.as_str();
            let b = b.as_str();
            let a = &map[a][0];
            let b = &map[b][0];
            let a = &a["type_order"].as_u64().unwrap_or(0);
            let b = &b["type_order"].as_u64().unwrap_or(0);
            a.cmp(b)
        });

        let mut out_vec = Vec::new();

        for key in keys {
            out_vec.push((key, map[key].clone()));
        }

        Ok(to_value(out_vec).unwrap())
    }
}
