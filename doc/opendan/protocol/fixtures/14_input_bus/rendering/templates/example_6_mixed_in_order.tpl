{% for item in input.items %}
{% if item.is_msg %}{{ item | render_format: "message.xml" }}{% else %}{{ item | render_format: "event.xml" }}{% endif %}
{% endfor %}
