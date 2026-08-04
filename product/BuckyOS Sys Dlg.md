# BuckyOS Sys Dlg

System Dialog 是系统提供的一组通用功能dlg,参数稳定，任何应用都可以通过触发调用来实现功能。 有2种使用方法
1）直接跳转（在新窗口种打开)
2）用iframe拉起(适合应用内部集成)

使用上，分模态（期待返回值）和非模态（只拉起不关心最终结果）

简单列表如下

## sysdlg/app_installer? 

安装app

## sysdlg/share

分享NamedObject （文件、对象...）

## sysdlg/select

选择一个系统中已经保存的 File / NamedObject

## sysdlg/request_do

请求执行一个action: 授权、支付、签名等

